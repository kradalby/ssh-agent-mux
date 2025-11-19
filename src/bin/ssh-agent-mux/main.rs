use color_eyre::eyre::Result as EyreResult;
use ssh_agent_mux::{socket_manager::SocketManager, watcher, MuxAgent};
use std::sync::Arc;
use tokio::select;
use tokio::signal::{self, unix::SignalKind};
use tokio::sync::Mutex;

mod cli;
mod logging;
mod service;

const BUILD_VERSION: &str = env!("SSH_AGENT_MUX_BUILD_VERSION");
const GIT_DESCRIBE: &str = env!("SSH_AGENT_MUX_GIT_DESCRIBE");

#[cfg(debug_assertions)]
fn install_eyre_hook() -> EyreResult<()> {
    color_eyre::config::HookBuilder::default()
        .display_env_section(true)
        .install()
}

#[cfg(not(debug_assertions))]
fn install_eyre_hook() -> EyreResult<()> {
    color_eyre::config::HookBuilder::default()
        .display_env_section(false)
        .install()
}

// Use current_thread to keep our resource utilization down; this program will generally be
// accessed by only one user, at the start of each SSH session, so it doesn't need tokio's powerful
// async multithreading
#[tokio::main(flavor = "current_thread")]
async fn main() -> EyreResult<()> {
    install_eyre_hook()?;

    let mut config = cli::Config::parse()?;

    // LoggerHandle must be held until program termination so file logging takes place
    let _logger = logging::setup_logger(config.log_level.into(), config.log_file.as_deref())?;
    log::info!(
        "Starting ssh-agent-mux version {}; commit {}",
        BUILD_VERSION,
        GIT_DESCRIBE
    );

    if config.service.any() {
        return service::handle_service_command(&config);
    }

    let mut sigterm = signal::unix::signal(SignalKind::terminate())?;
    let mut sighup = signal::unix::signal(SignalKind::hangup())?;

    // Create shared socket manager
    let socket_manager = Arc::new(Mutex::new(SocketManager::new(
        config.agent_sock_paths.clone(),
    )));

    // Start file watcher if enabled
    let _watcher = if config.watch_for_ssh_forward {
        log::info!("SSH forwarding watch enabled");

        // Scan for existing forwarded agents
        match watcher::scan_existing_agents().await {
            Ok(agents) => {
                log::info!("Found {} existing SSH forwarded agents", agents.len());
                let mut manager = socket_manager.lock().await;
                for agent in agents {
                    manager.add_watched(agent);
                }
            }
            Err(e) => {
                log::warn!("Failed to scan for existing agents: {}", e);
            }
        }

        // Start watching for new agents
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let manager_clone = socket_manager.clone();

        // Spawn watcher task
        let watcher_result = watcher::watch_tmp_directory(tx.clone()).await;
        let watcher_handle = match watcher_result {
            Ok(w) => Some(w),
            Err(e) => {
                log::error!("Failed to start file watcher: {}", e);
                None
            }
        };

        // Spawn event handler task
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let mut manager = manager_clone.lock().await;
                match event {
                    watcher::WatchEvent::Added(path) => {
                        if manager.add_watched(path.clone()) {
                            log::info!("Added forwarded agent: {}", path.display());
                        }
                    }
                    watcher::WatchEvent::Removed(path) => {
                        if manager.remove_watched(&path) {
                            log::info!("Removed forwarded agent: {}", path.display());
                        }
                    }
                }
            }
        });

        watcher_handle
    } else {
        None
    };

    // Run the mux agent with shared socket manager
    let listen_sock = config.listen_path.clone();

    loop {
        select! {
            res = MuxAgent::run_with_manager(&listen_sock, socket_manager.clone()) => { res?; break },
            // Cleanly exit on interrupt and SIGTERM, allowing
            // MuxAgent to clean up
            _ = signal::ctrl_c() => { log::info!("Exiting on SIGINT"); break },
            Some(_) = sigterm.recv() => { log::info!("Exiting on SIGTERM"); break },
            Some(_) = sighup.recv() => {
                log::info!("Reloading configuration");
                config = cli::Config::parse()?;
                // Update socket manager with new configured sockets
                let mut manager = socket_manager.lock().await;
                manager.update_configured(config.agent_sock_paths.clone());
            }
        }
    }

    Ok(())
}

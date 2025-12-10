use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::Result as EyreResult;
use ssh_agent_mux::control::{
    ControlServer, ControlServerState, SelfDeletingControlSocket, WatcherStatus,
};
use ssh_agent_mux::{socket_manager::SocketManager, watcher, MuxAgent};
use tokio::select;
use tokio::signal::{self, unix::SignalKind};
use tokio::sync::Mutex;

mod cli;
mod commands;
mod logging;
mod service;
mod systemd;

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

fn main() -> ExitCode {
    // Install eyre hook for nice error formatting
    if let Err(e) = install_eyre_hook() {
        eprintln!("Failed to install error hook: {}", e);
        return ExitCode::FAILURE;
    }

    // Parse CLI arguments
    let args = cli::Args::parse();

    // Check if we're running a client command
    if let Some(ref command) = args.command {
        // For client commands, we just need the control socket path
        let control_socket = args
            .control_socket
            .clone()
            .unwrap_or_else(|| cli::derive_control_path(&args.config_path));

        // Determine output format
        let format = if args.json {
            commands::OutputFormat::Json
        } else {
            commands::OutputFormat::Human
        };

        return commands::run_command(command, &control_socket, format);
    }

    // Run the daemon
    match run_daemon() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

// Use current_thread to keep our resource utilization down; this program will generally be
// accessed by only one user, at the start of each SSH session, so it doesn't need tokio's powerful
// async multithreading
#[tokio::main(flavor = "current_thread")]
async fn run_daemon() -> EyreResult<()> {
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

    // Track watcher status
    let mut watcher_status = if config.watch_for_ssh_forward {
        WatcherStatus::Active
    } else {
        WatcherStatus::Disabled
    };

    // Create shutdown channel for polling fallback
    let (shutdown_tx, _shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

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

        // Try smart watcher with automatic fallback
        let watch_result = watcher::start_watching(tx.clone()).await;

        // Update watcher status based on result
        let watcher_handle = match watch_result.mode {
            watcher::WatchMode::Smart(w) => {
                log::info!("Smart file watcher started successfully");
                watcher_status = WatcherStatus::Active;
                Some(w)
            }
            watcher::WatchMode::Polling => {
                let reason = watch_result
                    .fallback_reason
                    .unwrap_or_else(|| "Unknown error".to_string());
                log::warn!("Using polling fallback: {}", reason);
                watcher_status = WatcherStatus::PollingFallback(reason);

                // Start the polling loop
                let poll_interval = Duration::from_secs(30); // Default 30s polling
                let shutdown_rx = shutdown_tx.subscribe();
                tokio::spawn(watcher::run_polling_loop(tx.clone(), poll_interval, shutdown_rx));

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

    // Determine health check interval:
    // - If systemd watchdog is enabled, use half the watchdog timeout
    // - Otherwise use the configured interval (if any)
    // This ensures watchdog pings happen after real health checks
    let health_interval = if let Some(watchdog_usec) = systemd::watchdog_enabled() {
        let watchdog_interval = Duration::from_micros(watchdog_usec / 2);
        log::info!(
            "systemd watchdog enabled, health check interval: {:?}",
            watchdog_interval
        );
        Some(watchdog_interval)
    } else if config.health_check_interval > 0 {
        Some(Duration::from_secs(config.health_check_interval))
    } else {
        None
    };

    // Start health check task that also pings systemd watchdog
    if let Some(interval) = health_interval {
        let manager = socket_manager.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the first tick (immediate)
            ticker.tick().await;

            loop {
                ticker.tick().await;

                // Run actual health check
                let mut mgr = manager.lock().await;
                let removed = mgr.validate_and_cleanup();
                if !removed.is_empty() {
                    log::info!(
                        "Health check removed {} stale socket(s)",
                        removed.len()
                    );
                }
                drop(mgr);

                // Ping watchdog after successful health check
                systemd::notify_watchdog();
            }
        });

        log::info!("Health check task started (interval: {:?})", interval);
    }

    // Get paths for sockets
    let listen_sock = config.listen_path.clone();
    let control_sock = config.get_control_socket_path();

    // Create control server state
    let control_state = Arc::new(ControlServerState {
        socket_manager: socket_manager.clone(),
        listen_path: listen_sock.clone(),
        control_path: control_sock.clone(),
        watch_enabled: config.watch_for_ssh_forward,
        watcher_status,
        version: BUILD_VERSION.to_string(),
        git_commit: GIT_DESCRIBE.to_string(),
        pid: std::process::id(),
    });

    // Start control server
    let control_server = ControlServer::bind(&control_sock, control_state).await?;

    // Create self-deleting wrapper for control socket cleanup
    let _control_socket_cleanup = SelfDeletingControlSocket::new(control_sock.clone());

    log::info!("Control server listening on {}", control_sock.display());

    // Spawn control server task
    tokio::spawn(async move {
        if let Err(e) = control_server.run().await {
            log::error!("Control server error: {}", e);
        }
    });

    // Notify systemd that we're ready (for Type=notify services)
    systemd::notify_ready();
    systemd::notify_status("Running");

    // Run the mux agent with shared socket manager
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

// Re-export Args::parse for cli module
impl cli::Args {
    pub fn parse() -> Self {
        <Self as clap_serde_derive::clap::Parser>::parse()
    }
}

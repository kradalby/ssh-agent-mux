//! CLI command handlers for interacting with the running daemon.

use std::path::PathBuf;
use std::process::ExitCode;

use ssh_agent_mux::control::{
    ControlClient, HealthCheckResult, SocketHealthStatus, SocketInfo, StatusInfo,
};

/// Output format for CLI commands
pub enum OutputFormat {
    Human,
    Json,
}

/// Run a CLI command against the daemon
pub fn run_command(
    command: &crate::cli::Command,
    control_socket: &PathBuf,
    format: OutputFormat,
) -> ExitCode {
    let mut client = match ControlClient::connect(control_socket) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Failed to connect to daemon: {}", e);
            eprintln!("Is ssh-agent-mux running?");
            eprintln!("Control socket: {}", control_socket.display());
            return ExitCode::FAILURE;
        }
    };

    match command {
        crate::cli::Command::Serve { .. } => {
            // Should never reach here - serve is handled in main
            unreachable!("Serve command should be handled in main")
        }
        crate::cli::Command::Status => cmd_status(&mut client, format),
        crate::cli::Command::List => cmd_list(&mut client, format),
        crate::cli::Command::ListKeys => cmd_list_keys(&mut client, format),
        crate::cli::Command::Reload => cmd_reload(&mut client, format),
        crate::cli::Command::Validate => cmd_validate(&mut client, format),
        crate::cli::Command::Add { path } => cmd_add(&mut client, path, format),
        crate::cli::Command::Remove { path } => cmd_remove(&mut client, path, format),
        crate::cli::Command::Health => cmd_health(&mut client, format),
    }
}

fn cmd_status(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.status() {
        Ok(status) => {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&status).unwrap());
                }
                OutputFormat::Human => {
                    print_status_human(&status);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_status_human(status: &StatusInfo) {
    println!("ssh-agent-mux v{} ({})", status.version, status.git_commit);
    println!("  PID:            {}", status.pid);
    println!("  Uptime:         {}", format_duration(status.uptime_secs));
    println!();
    println!("Sockets:");
    println!("  Agent:          {}", status.listening_on);
    println!("  Control:        {}", status.control_socket);
    println!();
    println!("Watch:");
    println!("  Enabled:        {}", if status.watch_enabled { "yes" } else { "no" });
    println!("  Status:         {}", status.watcher_status);
    println!();
    println!("Stats:");
    println!("  Upstream:       {} socket(s)", status.socket_count);
    if let Some(keys) = status.key_count {
        println!("  Keys:           {} available", keys);
    }
}

fn cmd_list(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.list_sockets() {
        Ok(sockets) => {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&sockets).unwrap());
                }
                OutputFormat::Human => {
                    print_sockets_human(&sockets);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_sockets_human(sockets: &[SocketInfo]) {
    if sockets.is_empty() {
        println!("No upstream agent sockets configured.");
        return;
    }

    // Header
    println!(
        "{:<6} {:<12} {:<8} {:<20} {}",
        "ORDER", "SOURCE", "HEALTHY", "ADDED", "PATH"
    );

    for socket in sockets {
        let added = socket
            .added_at
            .as_ref()
            .map(|s| format_timestamp(s))
            .unwrap_or_else(|| "-".to_string());

        let healthy = if socket.healthy { "yes" } else { "no" };

        println!(
            "{:<6} {:<12} {:<8} {:<20} {}",
            socket.order,
            socket.source,
            healthy,
            added,
            socket.path
        );
    }
}

fn cmd_list_keys(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.list_keys() {
        Ok(keys) => {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&keys).unwrap());
                }
                OutputFormat::Human => {
                    if keys.is_empty() {
                        println!("No keys available.");
                    } else {
                        println!(
                            "{:<50} {:<10} {:<30} {}",
                            "FINGERPRINT", "TYPE", "COMMENT", "SOURCE"
                        );
                        for key in &keys {
                            // Truncate fingerprint for display
                            let fp = if key.fingerprint.len() > 47 {
                                format!("{}...", &key.fingerprint[..47])
                            } else {
                                key.fingerprint.clone()
                            };
                            let comment = if key.comment.len() > 27 {
                                format!("{}...", &key.comment[..27])
                            } else {
                                key.comment.clone()
                            };
                            println!(
                                "{:<50} {:<10} {:<30} {}",
                                fp, key.key_type, comment, key.source_socket
                            );
                        }
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_reload(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.reload() {
        Ok(message) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": true,
                            "message": message
                        })
                    );
                }
                OutputFormat::Human => {
                    println!("{}", message);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": false,
                            "error": e.to_string()
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", e);
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_validate(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.validate() {
        Ok(message) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": true,
                            "message": message
                        })
                    );
                }
                OutputFormat::Human => {
                    println!("{}", message);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": false,
                            "error": e.to_string()
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", e);
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_add(client: &mut ControlClient, path: &PathBuf, format: OutputFormat) -> ExitCode {
    match client.add_socket(&path.display().to_string()) {
        Ok(message) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": true,
                            "message": message
                        })
                    );
                }
                OutputFormat::Human => {
                    println!("{}", message);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": false,
                            "error": e.to_string()
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", e);
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_remove(client: &mut ControlClient, path: &PathBuf, format: OutputFormat) -> ExitCode {
    match client.remove_socket(&path.display().to_string()) {
        Ok(message) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": true,
                            "message": message
                        })
                    );
                }
                OutputFormat::Human => {
                    println!("{}", message);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": false,
                            "error": e.to_string()
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", e);
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_health(client: &mut ControlClient, format: OutputFormat) -> ExitCode {
    match client.health_check() {
        Ok(result) => {
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
                OutputFormat::Human => {
                    print_health_human(&result);
                }
            }

            // Exit with failure if any sockets are unhealthy
            if result.unhealthy_count > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": false,
                            "error": e.to_string()
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", e);
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn print_health_human(result: &HealthCheckResult) {
    println!(
        "Checking {} socket(s)...",
        result.sockets.len()
    );

    for (i, socket) in result.sockets.iter().enumerate() {
        let status_icon = match socket.status {
            SocketHealthStatus::Healthy => "✓",
            _ => "✗",
        };

        println!(
            "  [{}/{}] {}",
            i + 1,
            result.sockets.len(),
            socket.path
        );
        println!(
            "        Status: {} {}",
            status_icon,
            socket.status
        );

        if let Some(count) = socket.key_count {
            println!("        Keys: {}", count);
        }

        if let Some(ref error) = socket.error {
            println!("        Error: {}", error);
        }
    }

    println!();
    if result.unhealthy_count == 0 {
        println!("All sockets healthy.");
    } else {
        println!(
            "{} healthy, {} unhealthy",
            result.healthy_count, result.unhealthy_count
        );
    }

    if !result.removed.is_empty() {
        println!();
        println!("Removed {} stale socket(s):", result.removed.len());
        for path in &result.removed {
            println!("  - {}", path);
        }
    }
}

/// Format a duration in seconds as human-readable
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    } else {
        let days = secs / 86400;
        let hours = (secs % 86400) / 3600;
        format!("{}d {}h", days, hours)
    }
}

/// Format an ISO 8601 timestamp for display
fn format_timestamp(iso: &str) -> String {
    // Try to parse and format nicely, fall back to original
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

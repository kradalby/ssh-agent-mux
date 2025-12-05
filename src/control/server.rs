//! Control server that listens on a Unix socket for management commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::control::protocol::*;
use crate::socket_manager::SocketManager;
use crate::watcher;

/// Shared state for the control server
pub struct ControlServerState {
    /// Socket manager (shared with MuxAgent)
    pub socket_manager: Arc<Mutex<SocketManager>>,
    /// Path to the SSH agent listen socket
    pub listen_path: PathBuf,
    /// Path to the control socket
    pub control_path: PathBuf,
    /// Whether SSH forwarding watch is enabled
    pub watch_enabled: bool,
    /// Current watcher status
    pub watcher_status: WatcherStatus,
    /// Software version
    pub version: String,
    /// Git commit
    pub git_commit: String,
    /// Process ID
    pub pid: u32,
}

/// Control server that accepts commands over a Unix socket
pub struct ControlServer {
    listener: UnixListener,
    state: Arc<ControlServerState>,
}

impl ControlServer {
    /// Bind a new control server to the given path
    pub async fn bind(
        control_path: impl AsRef<Path>,
        state: Arc<ControlServerState>,
    ) -> std::io::Result<Self> {
        let control_path = control_path.as_ref();

        // Remove existing socket if present
        if control_path.exists() {
            std::fs::remove_file(control_path)?;
        }

        // Ensure parent directory exists
        if let Some(parent) = control_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(control_path)?;
        log::info!(
            "Control server listening on {}",
            control_path.display()
        );

        Ok(Self { listener, state })
    }

    /// Run the control server, accepting and handling connections
    pub async fn run(&self) -> std::io::Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let state = self.state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state).await {
                            log::warn!("Error handling control connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    log::error!("Error accepting control connection: {}", e);
                }
            }
        }
    }

    /// Accept a single connection (useful for testing)
    pub async fn accept_one(&self) -> std::io::Result<()> {
        let (stream, _) = self.listener.accept().await?;
        handle_connection(stream, self.state.clone()).await
    }
}

/// Handle a single control connection
async fn handle_connection(
    stream: UnixStream,
    state: Arc<ControlServerState>,
) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read requests line by line
    while reader.read_line(&mut line).await? > 0 {
        let request: ControlRequest = match serde_json::from_str(line.trim()) {
            Ok(req) => req,
            Err(e) => {
                let response = ControlResponse::Error {
                    error: format!("Invalid request: {}", e),
                };
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                line.clear();
                continue;
            }
        };

        log::debug!("Control request: {:?}", request);
        let response = handle_request(request, &state).await;
        log::debug!("Control response: {:?}", response);

        let response_json = serde_json::to_string(&response)?;
        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        line.clear();
    }

    Ok(())
}

/// Handle a single control request
async fn handle_request(
    request: ControlRequest,
    state: &ControlServerState,
) -> ControlResponse {
    match request {
        ControlRequest::Ping => ControlResponse::Pong,

        ControlRequest::Status => {
            let manager = state.socket_manager.lock().await;
            ControlResponse::Status(StatusInfo {
                version: state.version.clone(),
                git_commit: state.git_commit.clone(),
                uptime_secs: manager.uptime_secs(),
                pid: state.pid,
                listening_on: state.listen_path.display().to_string(),
                control_socket: state.control_path.display().to_string(),
                watch_enabled: state.watch_enabled,
                watcher_status: state.watcher_status.clone(),
                socket_count: manager.total_count(),
                key_count: None, // Would need to query upstream agents
            })
        }

        ControlRequest::ListSockets => {
            let manager = state.socket_manager.lock().await;
            ControlResponse::Sockets {
                sockets: manager.get_socket_info(),
            }
        }

        ControlRequest::ListKeys => {
            // This would require connecting to each upstream agent and querying keys
            // For now, return an error indicating this isn't implemented yet
            ControlResponse::Error {
                error: "ListKeys not yet implemented - requires upstream agent queries".to_string(),
            }
        }

        ControlRequest::Reload => {
            if !state.watch_enabled {
                return ControlResponse::Error {
                    error: "SSH forwarding watch is not enabled".to_string(),
                };
            }

            // Scan for existing agents
            match watcher::scan_existing_agents().await {
                Ok(agents) => {
                    let mut manager = state.socket_manager.lock().await;
                    let mut added = 0;
                    for agent in agents {
                        if manager.add_watched(agent) {
                            added += 1;
                        }
                    }

                    // Also cleanup stale sockets
                    let removed = manager.validate_and_cleanup();

                    ControlResponse::Success {
                        message: Some(format!(
                            "Reload complete: {} added, {} removed",
                            added,
                            removed.len()
                        )),
                    }
                }
                Err(e) => ControlResponse::Error {
                    error: format!("Failed to scan for agents: {}", e),
                },
            }
        }

        ControlRequest::ValidateSockets => {
            let mut manager = state.socket_manager.lock().await;
            let removed = manager.validate_and_cleanup();

            if removed.is_empty() {
                ControlResponse::Success {
                    message: Some("All sockets healthy".to_string()),
                }
            } else {
                ControlResponse::Success {
                    message: Some(format!(
                        "Removed {} stale socket(s): {}",
                        removed.len(),
                        removed
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                }
            }
        }

        ControlRequest::AddSocket { path } => {
            let path = PathBuf::from(&path);

            // Validate the socket exists
            if !path.exists() {
                return ControlResponse::Error {
                    error: format!("Socket does not exist: {}", path.display()),
                };
            }

            let mut manager = state.socket_manager.lock().await;

            // Check if already tracked
            if manager.is_watched(&path) || manager.is_configured(&path) {
                return ControlResponse::Error {
                    error: format!("Socket already tracked: {}", path.display()),
                };
            }

            if manager.add_watched(path.clone()) {
                ControlResponse::Success {
                    message: Some(format!("Added socket: {}", path.display())),
                }
            } else {
                ControlResponse::Error {
                    error: format!("Failed to add socket: {}", path.display()),
                }
            }
        }

        ControlRequest::RemoveSocket { path } => {
            let path = PathBuf::from(&path);
            let mut manager = state.socket_manager.lock().await;

            // Can only remove watched sockets, not configured ones
            if manager.is_configured(&path) {
                return ControlResponse::Error {
                    error: format!(
                        "Cannot remove configured socket: {} (edit config file instead)",
                        path.display()
                    ),
                };
            }

            if manager.remove_watched(&path) {
                ControlResponse::Success {
                    message: Some(format!("Removed socket: {}", path.display())),
                }
            } else {
                ControlResponse::Error {
                    error: format!("Socket not found in watched list: {}", path.display()),
                }
            }
        }

        ControlRequest::HealthCheck => {
            let manager = state.socket_manager.lock().await;
            let sockets = manager.get_ordered_sockets();
            drop(manager); // Release lock during health checks

            let mut results = Vec::new();
            let mut healthy_count = 0;
            let mut unhealthy_count = 0;

            for socket_path in &sockets {
                let (status, key_count, error) = check_socket_health(socket_path).await;

                if status == SocketHealthStatus::Healthy {
                    healthy_count += 1;
                } else {
                    unhealthy_count += 1;
                }

                results.push(SocketHealthInfo {
                    path: socket_path.display().to_string(),
                    status,
                    key_count,
                    error,
                });
            }

            // Remove unhealthy sockets
            let mut manager = state.socket_manager.lock().await;
            let removed = manager.validate_and_cleanup();

            ControlResponse::HealthCheck(HealthCheckResult {
                sockets: results,
                healthy_count,
                unhealthy_count,
                removed: removed.iter().map(|p| p.display().to_string()).collect(),
            })
        }
    }
}

/// Check the health of a single socket
async fn check_socket_health(path: &Path) -> (SocketHealthStatus, Option<usize>, Option<String>) {
    // Check if file exists
    if !path.exists() {
        return (SocketHealthStatus::Missing, None, Some("Socket file does not exist".to_string()));
    }

    // Try to connect
    let stream = match std::os::unix::net::UnixStream::connect(path) {
        Ok(s) => s,
        Err(e) => {
            return (
                SocketHealthStatus::ConnectionFailed,
                None,
                Some(format!("Connection failed: {}", e)),
            );
        }
    };

    // Try to create a client using ssh-agent-lib
    // This validates the socket responds to the SSH agent protocol
    use ssh_agent_lib::client;
    match client::connect(stream.into()) {
        Ok(_client) => {
            // Successfully connected and established protocol
            // Note: We could query keys here with _client.request_identities().await
            // but that requires more async refactoring. For now, a successful
            // connection is sufficient for health checking.
            (SocketHealthStatus::Healthy, None, None)
        }
        Err(e) => {
            (
                SocketHealthStatus::ProtocolError,
                None,
                Some(format!("Protocol error: {}", e)),
            )
        }
    }
}

/// Self-deleting Unix listener for the control socket
pub struct SelfDeletingControlSocket {
    path: PathBuf,
}

impl SelfDeletingControlSocket {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SelfDeletingControlSocket {
    fn drop(&mut self) {
        log::debug!("Cleaning up control socket {}", self.path.display());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_control_server_ping() {
        let temp_dir = TempDir::new().unwrap();
        let control_path = temp_dir.path().join("test.ctl");
        let listen_path = temp_dir.path().join("test.sock");

        let socket_manager = Arc::new(Mutex::new(SocketManager::new(vec![])));

        let state = Arc::new(ControlServerState {
            socket_manager,
            listen_path: listen_path.clone(),
            control_path: control_path.clone(),
            watch_enabled: false,
            watcher_status: WatcherStatus::Disabled,
            version: "test".to_string(),
            git_commit: "test".to_string(),
            pid: std::process::id(),
        });

        let server = ControlServer::bind(&control_path, state).await.unwrap();

        // Spawn server in background
        let server_handle = tokio::spawn(async move {
            server.accept_one().await
        });

        // Give server time to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Connect as client
        let mut stream = UnixStream::connect(&control_path).await.unwrap();

        // Send ping
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let request = serde_json::to_string(&ControlRequest::Ping).unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();

        // Read response
        let mut response = String::new();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await.unwrap();
        response.push_str(std::str::from_utf8(&buf[..n]).unwrap());

        let parsed: ControlResponse = serde_json::from_str(response.trim()).unwrap();
        assert_eq!(parsed, ControlResponse::Pong);

        // Close connection
        drop(stream);

        // Wait for server to finish
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_handle_status_request() {
        let socket_manager = Arc::new(Mutex::new(SocketManager::new(vec![])));

        let state = Arc::new(ControlServerState {
            socket_manager,
            listen_path: PathBuf::from("/test/listen.sock"),
            control_path: PathBuf::from("/test/control.ctl"),
            watch_enabled: true,
            watcher_status: WatcherStatus::Active,
            version: "1.0.0".to_string(),
            git_commit: "abc123".to_string(),
            pid: 12345,
        });

        let response = handle_request(ControlRequest::Status, &state).await;

        match response {
            ControlResponse::Status(info) => {
                assert_eq!(info.version, "1.0.0");
                assert_eq!(info.git_commit, "abc123");
                assert_eq!(info.pid, 12345);
                assert!(info.watch_enabled);
                assert_eq!(info.watcher_status, WatcherStatus::Active);
            }
            _ => panic!("Expected Status response"),
        }
    }

    #[tokio::test]
    async fn test_handle_list_sockets_request() {
        let mut manager = SocketManager::new(vec![PathBuf::from("/tmp/configured.sock")]);
        manager.add_watched(PathBuf::from("/tmp/watched.sock"));

        let state = Arc::new(ControlServerState {
            socket_manager: Arc::new(Mutex::new(manager)),
            listen_path: PathBuf::from("/test/listen.sock"),
            control_path: PathBuf::from("/test/control.ctl"),
            watch_enabled: false,
            watcher_status: WatcherStatus::Disabled,
            version: "test".to_string(),
            git_commit: "test".to_string(),
            pid: 1,
        });

        let response = handle_request(ControlRequest::ListSockets, &state).await;

        match response {
            ControlResponse::Sockets { sockets } => {
                assert_eq!(sockets.len(), 2);
                // Watched should be first
                assert_eq!(sockets[0].source, SocketSource::Watched);
                assert_eq!(sockets[1].source, SocketSource::Configured);
            }
            _ => panic!("Expected Sockets response"),
        }
    }

    #[tokio::test]
    async fn test_handle_add_remove_socket() {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        // Create a fake socket file
        std::fs::File::create(&socket_path).unwrap();

        let state = Arc::new(ControlServerState {
            socket_manager: Arc::new(Mutex::new(SocketManager::new(vec![]))),
            listen_path: PathBuf::from("/test/listen.sock"),
            control_path: PathBuf::from("/test/control.ctl"),
            watch_enabled: false,
            watcher_status: WatcherStatus::Disabled,
            version: "test".to_string(),
            git_commit: "test".to_string(),
            pid: 1,
        });

        // Add socket
        let response = handle_request(
            ControlRequest::AddSocket {
                path: socket_path.display().to_string(),
            },
            &state,
        )
        .await;

        match response {
            ControlResponse::Success { message } => {
                assert!(message.unwrap().contains("Added socket"));
            }
            _ => panic!("Expected Success response"),
        }

        // Verify it was added
        let manager = state.socket_manager.lock().await;
        assert!(manager.is_watched(&socket_path));
        drop(manager);

        // Try to add again (should fail)
        let response = handle_request(
            ControlRequest::AddSocket {
                path: socket_path.display().to_string(),
            },
            &state,
        )
        .await;

        match response {
            ControlResponse::Error { error } => {
                assert!(error.contains("already tracked"));
            }
            _ => panic!("Expected Error response"),
        }

        // Remove socket
        let response = handle_request(
            ControlRequest::RemoveSocket {
                path: socket_path.display().to_string(),
            },
            &state,
        )
        .await;

        match response {
            ControlResponse::Success { message } => {
                assert!(message.unwrap().contains("Removed socket"));
            }
            _ => panic!("Expected Success response"),
        }

        // Verify it was removed
        let manager = state.socket_manager.lock().await;
        assert!(!manager.is_watched(&socket_path));
    }
}

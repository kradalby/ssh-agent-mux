//! Control client for sending commands to the daemon.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::control::protocol::*;

/// Error type for control client operations
#[derive(Debug)]
pub enum ControlClientError {
    /// Failed to connect to control socket
    ConnectionFailed(std::io::Error),
    /// Failed to send request
    SendFailed(std::io::Error),
    /// Failed to receive response
    ReceiveFailed(std::io::Error),
    /// Failed to serialize request
    SerializeFailed(serde_json::Error),
    /// Failed to deserialize response
    DeserializeFailed(serde_json::Error),
    /// Connection timed out
    Timeout,
    /// Daemon returned an error
    DaemonError(String),
}

impl std::fmt::Display for ControlClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlClientError::ConnectionFailed(e) => {
                write!(f, "Failed to connect to control socket: {}", e)
            }
            ControlClientError::SendFailed(e) => write!(f, "Failed to send request: {}", e),
            ControlClientError::ReceiveFailed(e) => write!(f, "Failed to receive response: {}", e),
            ControlClientError::SerializeFailed(e) => write!(f, "Failed to serialize request: {}", e),
            ControlClientError::DeserializeFailed(e) => {
                write!(f, "Failed to deserialize response: {}", e)
            }
            ControlClientError::Timeout => write!(f, "Connection timed out"),
            ControlClientError::DaemonError(e) => write!(f, "Daemon error: {}", e),
        }
    }
}

impl std::error::Error for ControlClientError {}

/// Client for communicating with the control server
pub struct ControlClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl ControlClient {
    /// Connect to the control socket
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, ControlClientError> {
        Self::connect_with_timeout(path, Duration::from_secs(5))
    }

    /// Connect to the control socket with a custom timeout
    pub fn connect_with_timeout(
        path: impl AsRef<Path>,
        timeout: Duration,
    ) -> Result<Self, ControlClientError> {
        let path = path.as_ref();

        let stream =
            UnixStream::connect(path).map_err(ControlClientError::ConnectionFailed)?;

        stream
            .set_read_timeout(Some(timeout))
            .map_err(ControlClientError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(ControlClientError::ConnectionFailed)?;

        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(ControlClientError::ConnectionFailed)?,
        );

        Ok(Self { stream, reader })
    }

    /// Send a request and receive a response
    pub fn send(&mut self, request: ControlRequest) -> Result<ControlResponse, ControlClientError> {
        // Serialize and send request
        let request_json =
            serde_json::to_string(&request).map_err(ControlClientError::SerializeFailed)?;

        self.stream
            .write_all(request_json.as_bytes())
            .map_err(ControlClientError::SendFailed)?;
        self.stream
            .write_all(b"\n")
            .map_err(ControlClientError::SendFailed)?;
        self.stream.flush().map_err(ControlClientError::SendFailed)?;

        // Read response
        let mut response_line = String::new();
        self.reader
            .read_line(&mut response_line)
            .map_err(ControlClientError::ReceiveFailed)?;

        // Deserialize response
        let response: ControlResponse = serde_json::from_str(response_line.trim())
            .map_err(ControlClientError::DeserializeFailed)?;

        Ok(response)
    }

    /// Send a ping and verify the daemon is alive
    pub fn ping(&mut self) -> Result<(), ControlClientError> {
        match self.send(ControlRequest::Ping)? {
            ControlResponse::Pong => Ok(()),
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to ping".to_string(),
            )),
        }
    }

    /// Get daemon status
    pub fn status(&mut self) -> Result<StatusInfo, ControlClientError> {
        match self.send(ControlRequest::Status)? {
            ControlResponse::Status(info) => Ok(info),
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to status".to_string(),
            )),
        }
    }

    /// List all sockets
    pub fn list_sockets(&mut self) -> Result<Vec<SocketInfo>, ControlClientError> {
        match self.send(ControlRequest::ListSockets)? {
            ControlResponse::Sockets { sockets } => Ok(sockets),
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to list_sockets".to_string(),
            )),
        }
    }

    /// List all keys
    pub fn list_keys(&mut self) -> Result<Vec<KeyInfo>, ControlClientError> {
        match self.send(ControlRequest::ListKeys)? {
            ControlResponse::Keys { keys } => Ok(keys),
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to list_keys".to_string(),
            )),
        }
    }

    /// Reload (re-scan for forwarded agents)
    pub fn reload(&mut self) -> Result<String, ControlClientError> {
        match self.send(ControlRequest::Reload)? {
            ControlResponse::Success { message } => {
                Ok(message.unwrap_or_else(|| "Reload complete".to_string()))
            }
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to reload".to_string(),
            )),
        }
    }

    /// Validate sockets (remove stale ones)
    pub fn validate(&mut self) -> Result<String, ControlClientError> {
        match self.send(ControlRequest::ValidateSockets)? {
            ControlResponse::Success { message } => {
                Ok(message.unwrap_or_else(|| "Validation complete".to_string()))
            }
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to validate".to_string(),
            )),
        }
    }

    /// Add a socket to the watched list
    pub fn add_socket(&mut self, path: &str) -> Result<String, ControlClientError> {
        match self.send(ControlRequest::AddSocket {
            path: path.to_string(),
        })? {
            ControlResponse::Success { message } => {
                Ok(message.unwrap_or_else(|| "Socket added".to_string()))
            }
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to add_socket".to_string(),
            )),
        }
    }

    /// Remove a socket from the watched list
    pub fn remove_socket(&mut self, path: &str) -> Result<String, ControlClientError> {
        match self.send(ControlRequest::RemoveSocket {
            path: path.to_string(),
        })? {
            ControlResponse::Success { message } => {
                Ok(message.unwrap_or_else(|| "Socket removed".to_string()))
            }
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to remove_socket".to_string(),
            )),
        }
    }

    /// Perform a full health check
    pub fn health_check(&mut self) -> Result<HealthCheckResult, ControlClientError> {
        match self.send(ControlRequest::HealthCheck)? {
            ControlResponse::HealthCheck(result) => Ok(result),
            ControlResponse::Error { error } => Err(ControlClientError::DaemonError(error)),
            _ => Err(ControlClientError::DaemonError(
                "Unexpected response to health_check".to_string(),
            )),
        }
    }
}

/// Derive the default control socket path from the listen socket path
pub fn default_control_path(listen_path: &Path) -> std::path::PathBuf {
    let listen_str = listen_path.to_string_lossy();

    // Replace .sock with .ctl
    if listen_str.ends_with(".sock") {
        let base = &listen_str[..listen_str.len() - 5];
        std::path::PathBuf::from(format!("{}.ctl", base))
    } else {
        // Just append .ctl
        std::path::PathBuf::from(format!("{}.ctl", listen_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_control_path() {
        assert_eq!(
            default_control_path(&PathBuf::from("/home/user/.ssh/ssh-agent-mux.sock")),
            PathBuf::from("/home/user/.ssh/ssh-agent-mux.ctl")
        );

        assert_eq!(
            default_control_path(&PathBuf::from("/tmp/agent.sock")),
            PathBuf::from("/tmp/agent.ctl")
        );

        assert_eq!(
            default_control_path(&PathBuf::from("/tmp/agent")),
            PathBuf::from("/tmp/agent.ctl")
        );
    }

    #[test]
    fn test_error_display() {
        let err = ControlClientError::DaemonError("test error".to_string());
        assert_eq!(format!("{}", err), "Daemon error: test error");

        let err = ControlClientError::Timeout;
        assert_eq!(format!("{}", err), "Connection timed out");
    }
}

//! Control protocol types for communication between CLI client and daemon.
//!
//! The protocol uses JSON-over-Unix-socket with newline-delimited messages:
//! ```text
//! Client → Server: {"type": "Status"}\n
//! Server → Client: {"type": "Status", "data": {...}}\n
//! ```

use serde::{Deserialize, Serialize};

/// Request types sent from CLI client to daemon
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum ControlRequest {
    /// Get daemon status (uptime, version, config)
    Status,

    /// List all upstream agent sockets with metadata
    ListSockets,

    /// List all available SSH keys with source info
    ListKeys,

    /// Re-scan /tmp for forwarded agents
    Reload,

    /// Validate all sockets, remove unreachable ones
    ValidateSockets,

    /// Remove a specific socket from the watched list
    RemoveSocket { path: String },

    /// Add a socket to the watched list
    AddSocket { path: String },

    /// Full health check: validate + query keys from each socket
    HealthCheck,

    /// Ping (for connection testing / liveness check)
    Ping,
}

/// Response types sent from daemon to CLI client
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ControlResponse {
    /// Status information
    Status(StatusInfo),

    /// List of sockets
    Sockets { sockets: Vec<SocketInfo> },

    /// List of keys
    Keys { keys: Vec<KeyInfo> },

    /// Health check results
    HealthCheck(HealthCheckResult),

    /// Generic success with optional message
    Success { message: Option<String> },

    /// Error response
    Error { error: String },

    /// Pong response (reply to Ping)
    Pong,
}

/// Daemon status information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusInfo {
    /// Software version
    pub version: String,
    /// Git commit hash
    pub git_commit: String,
    /// Daemon uptime in seconds
    pub uptime_secs: u64,
    /// Process ID
    pub pid: u32,
    /// Path to the SSH agent listening socket
    pub listening_on: String,
    /// Path to the control socket
    pub control_socket: String,
    /// Whether SSH forwarding watch is enabled
    pub watch_enabled: bool,
    /// Current watcher status
    pub watcher_status: WatcherStatus,
    /// Number of upstream agent sockets
    pub socket_count: usize,
    /// Number of available SSH keys (if known)
    pub key_count: Option<usize>,
}

/// Status of the file watcher
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "reason")]
pub enum WatcherStatus {
    /// File watcher is active and healthy
    Active,
    /// File watcher failed, using polling fallback
    PollingFallback(String),
    /// Watching disabled in config
    Disabled,
}

/// Information about an upstream agent socket
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocketInfo {
    /// Path to the socket
    pub path: String,
    /// How this socket was added (configured vs watched)
    pub source: SocketSource,
    /// When this socket was added (ISO 8601 timestamp), None for configured sockets
    pub added_at: Option<String>,
    /// Whether the socket is currently healthy
    pub healthy: bool,
    /// Last health check time (ISO 8601 timestamp)
    pub last_health_check: Option<String>,
    /// Number of keys from this socket (if known)
    pub key_count: Option<usize>,
    /// Priority order (1 = highest priority)
    pub order: usize,
}

/// Source of a socket (how it was added)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SocketSource {
    /// Socket was configured in config file or CLI args
    Configured,
    /// Socket was detected by watching /tmp
    Watched,
}

/// Information about an SSH key
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyInfo {
    /// Key fingerprint (SHA256:...)
    pub fingerprint: String,
    /// Key type (ed25519, rsa, ecdsa, etc.)
    pub key_type: String,
    /// Key size in bits (if applicable)
    pub bits: Option<u32>,
    /// Key comment (usually user@host or description)
    pub comment: String,
    /// Path to the source socket that provides this key
    pub source_socket: String,
}

/// Result of a health check operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthCheckResult {
    /// Results for each socket
    pub sockets: Vec<SocketHealthInfo>,
    /// Total number of healthy sockets
    pub healthy_count: usize,
    /// Total number of unhealthy sockets
    pub unhealthy_count: usize,
    /// Sockets that were removed due to being unreachable
    pub removed: Vec<String>,
}

/// Health status of a single socket
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocketHealthInfo {
    /// Socket path
    pub path: String,
    /// Health status
    pub status: SocketHealthStatus,
    /// Number of keys (if healthy)
    pub key_count: Option<usize>,
    /// Error message (if unhealthy)
    pub error: Option<String>,
}

/// Health status of a socket
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocketHealthStatus {
    /// Socket is healthy and responding
    Healthy,
    /// Socket file is missing
    Missing,
    /// Socket exists but connection failed
    ConnectionFailed,
    /// Connected but protocol error occurred
    ProtocolError,
    /// Connected but failed to query keys
    QueryFailed,
}

impl std::fmt::Display for SocketHealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketHealthStatus::Healthy => write!(f, "healthy"),
            SocketHealthStatus::Missing => write!(f, "missing"),
            SocketHealthStatus::ConnectionFailed => write!(f, "connection failed"),
            SocketHealthStatus::ProtocolError => write!(f, "protocol error"),
            SocketHealthStatus::QueryFailed => write!(f, "query failed"),
        }
    }
}

impl std::fmt::Display for SocketSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketSource::Configured => write!(f, "configured"),
            SocketSource::Watched => write!(f, "watched"),
        }
    }
}

impl std::fmt::Display for WatcherStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatcherStatus::Active => write!(f, "active"),
            WatcherStatus::PollingFallback(reason) => write!(f, "polling ({})", reason),
            WatcherStatus::Disabled => write!(f, "disabled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization_status() {
        let req = ControlRequest::Status;
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"type":"Status"}"#);

        let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_request_serialization_ping() {
        let req = ControlRequest::Ping;
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"type":"Ping"}"#);

        let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_request_serialization_add_socket() {
        let req = ControlRequest::AddSocket {
            path: "/tmp/test.sock".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"type":"AddSocket","data":{"path":"/tmp/test.sock"}}"#);

        let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_request_serialization_remove_socket() {
        let req = ControlRequest::RemoveSocket {
            path: "/tmp/ssh-abc/agent.123".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("RemoveSocket"));
        assert!(json.contains("/tmp/ssh-abc/agent.123"));

        let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_response_serialization_pong() {
        let resp = ControlResponse::Pong;
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"type":"Pong"}"#);

        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_response_serialization_error() {
        let resp = ControlResponse::Error {
            error: "Something went wrong".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Something went wrong"));

        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_response_serialization_success() {
        let resp = ControlResponse::Success {
            message: Some("Operation completed".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Success"));
        assert!(json.contains("Operation completed"));

        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_response_serialization_success_no_message() {
        let resp = ControlResponse::Success { message: None };
        let json = serde_json::to_string(&resp).unwrap();

        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_status_info_serialization() {
        let status = StatusInfo {
            version: "0.2.0".to_string(),
            git_commit: "abc1234".to_string(),
            uptime_secs: 3600,
            pid: 12345,
            listening_on: "/home/user/.ssh/ssh-agent-mux.sock".to_string(),
            control_socket: "/home/user/.ssh/ssh-agent-mux.ctl".to_string(),
            watch_enabled: true,
            watcher_status: WatcherStatus::Active,
            socket_count: 2,
            key_count: Some(3),
        };

        let resp = ControlResponse::Status(status.clone());
        let json = serde_json::to_string(&resp).unwrap();

        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_watcher_status_serialization() {
        // Active
        let status = WatcherStatus::Active;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"status":"Active"}"#);

        // Disabled
        let status = WatcherStatus::Disabled;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"status":"Disabled"}"#);

        // PollingFallback
        let status = WatcherStatus::PollingFallback("Permission denied".to_string());
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("PollingFallback"));
        assert!(json.contains("Permission denied"));
    }

    #[test]
    fn test_socket_info_serialization() {
        let socket = SocketInfo {
            path: "/tmp/auth-agent123/listener.sock".to_string(),
            source: SocketSource::Watched,
            added_at: Some("2024-12-05T13:28:10Z".to_string()),
            healthy: true,
            last_health_check: Some("2024-12-05T14:00:00Z".to_string()),
            key_count: Some(2),
            order: 1,
        };

        let json = serde_json::to_string(&socket).unwrap();
        let parsed: SocketInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, socket);
    }

    #[test]
    fn test_socket_source_lowercase() {
        let configured = SocketSource::Configured;
        let json = serde_json::to_string(&configured).unwrap();
        assert_eq!(json, r#""configured""#);

        let watched = SocketSource::Watched;
        let json = serde_json::to_string(&watched).unwrap();
        assert_eq!(json, r#""watched""#);
    }

    #[test]
    fn test_key_info_serialization() {
        let key = KeyInfo {
            fingerprint: "SHA256:abc123def456".to_string(),
            key_type: "ed25519".to_string(),
            bits: None,
            comment: "user@laptop".to_string(),
            source_socket: "/tmp/auth-agent123/listener.sock".to_string(),
        };

        let json = serde_json::to_string(&key).unwrap();
        let parsed: KeyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, key);
    }

    #[test]
    fn test_health_check_result_serialization() {
        let result = HealthCheckResult {
            sockets: vec![
                SocketHealthInfo {
                    path: "/tmp/agent1.sock".to_string(),
                    status: SocketHealthStatus::Healthy,
                    key_count: Some(2),
                    error: None,
                },
                SocketHealthInfo {
                    path: "/tmp/agent2.sock".to_string(),
                    status: SocketHealthStatus::ConnectionFailed,
                    key_count: None,
                    error: Some("Connection refused".to_string()),
                },
            ],
            healthy_count: 1,
            unhealthy_count: 1,
            removed: vec![],
        };

        let resp = ControlResponse::HealthCheck(result.clone());
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_socket_health_status_snake_case() {
        let healthy = SocketHealthStatus::Healthy;
        let json = serde_json::to_string(&healthy).unwrap();
        assert_eq!(json, r#""healthy""#);

        let connection_failed = SocketHealthStatus::ConnectionFailed;
        let json = serde_json::to_string(&connection_failed).unwrap();
        assert_eq!(json, r#""connection_failed""#);

        let protocol_error = SocketHealthStatus::ProtocolError;
        let json = serde_json::to_string(&protocol_error).unwrap();
        assert_eq!(json, r#""protocol_error""#);
    }

    #[test]
    fn test_sockets_response() {
        let resp = ControlResponse::Sockets {
            sockets: vec![
                SocketInfo {
                    path: "/tmp/sock1".to_string(),
                    source: SocketSource::Watched,
                    added_at: Some("2024-12-05T10:00:00Z".to_string()),
                    healthy: true,
                    last_health_check: None,
                    key_count: Some(1),
                    order: 1,
                },
                SocketInfo {
                    path: "/home/user/.agent.sock".to_string(),
                    source: SocketSource::Configured,
                    added_at: None,
                    healthy: true,
                    last_health_check: None,
                    key_count: Some(2),
                    order: 2,
                },
            ],
        };

        let json = serde_json::to_string(&resp).unwrap();
        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_keys_response() {
        let resp = ControlResponse::Keys {
            keys: vec![
                KeyInfo {
                    fingerprint: "SHA256:abc".to_string(),
                    key_type: "ed25519".to_string(),
                    bits: None,
                    comment: "key1".to_string(),
                    source_socket: "/tmp/sock1".to_string(),
                },
                KeyInfo {
                    fingerprint: "SHA256:def".to_string(),
                    key_type: "rsa".to_string(),
                    bits: Some(4096),
                    comment: "key2".to_string(),
                    source_socket: "/tmp/sock2".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&resp).unwrap();
        let parsed: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn test_all_requests_roundtrip() {
        let requests = vec![
            ControlRequest::Status,
            ControlRequest::ListSockets,
            ControlRequest::ListKeys,
            ControlRequest::Reload,
            ControlRequest::ValidateSockets,
            ControlRequest::RemoveSocket {
                path: "/test".to_string(),
            },
            ControlRequest::AddSocket {
                path: "/test".to_string(),
            },
            ControlRequest::HealthCheck,
            ControlRequest::Ping,
        ];

        for req in requests {
            let json = serde_json::to_string(&req).unwrap();
            let parsed: ControlRequest = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, req, "Failed roundtrip for {:?}", req);
        }
    }
}

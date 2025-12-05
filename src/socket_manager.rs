use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use crate::control::{SocketInfo, SocketSource};

/// Manages both configured and watched sockets with proper ordering
#[derive(Debug, Clone)]
pub struct SocketManager {
    configured_sockets: Vec<PathBuf>,
    watched_sockets: HashMap<PathBuf, WatchedSocket>,
    /// When the daemon started (for uptime calculation)
    daemon_start_time: SystemTime,
    /// Last time a health check was performed
    last_health_check: Option<SystemTime>,
}

/// Represents a watched socket with metadata
#[derive(Debug, Clone)]
pub struct WatchedSocket {
    path: PathBuf,
    created_at: SystemTime,
    /// Whether socket was healthy at last check
    last_healthy: Option<bool>,
    /// Last time this socket was health checked
    last_health_check: Option<SystemTime>,
    /// Number of keys from this socket (if known)
    key_count: Option<usize>,
}

impl WatchedSocket {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            created_at: SystemTime::now(),
            last_healthy: None,
            last_health_check: None,
            key_count: None,
        }
    }
}

impl SocketManager {
    /// Create a new SocketManager with configured sockets
    pub fn new(configured_sockets: Vec<PathBuf>) -> Self {
        let manager = Self {
            configured_sockets,
            watched_sockets: HashMap::new(),
            daemon_start_time: SystemTime::now(),
            last_health_check: None,
        };
        manager.log_state("Initialized socket manager");
        manager
    }

    /// Get the daemon start time
    pub fn daemon_start_time(&self) -> SystemTime {
        self.daemon_start_time
    }

    /// Get uptime in seconds
    pub fn uptime_secs(&self) -> u64 {
        self.daemon_start_time
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Get ordered list of sockets: watched (newest first) + configured
    pub fn get_ordered_sockets(&self) -> Vec<PathBuf> {
        let mut result = Vec::new();

        // Add watched sockets sorted by newest first
        let mut watched: Vec<_> = self.watched_sockets.values().collect();
        watched.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result.extend(watched.iter().map(|s| s.path.clone()));

        // Add configured sockets in order
        result.extend(self.configured_sockets.iter().cloned());

        result
    }

    /// Get detailed socket information for all sockets
    pub fn get_socket_info(&self) -> Vec<SocketInfo> {
        let mut result = Vec::new();
        let mut order = 1;

        // Add watched sockets sorted by newest first
        let mut watched: Vec<_> = self.watched_sockets.values().collect();
        watched.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        for socket in watched {
            result.push(SocketInfo {
                path: socket.path.display().to_string(),
                source: SocketSource::Watched,
                added_at: Some(format_system_time(socket.created_at)),
                healthy: socket.last_healthy.unwrap_or(socket.path.exists()),
                last_health_check: socket.last_health_check.map(format_system_time),
                key_count: socket.key_count,
                order,
            });
            order += 1;
        }

        // Add configured sockets
        for path in &self.configured_sockets {
            result.push(SocketInfo {
                path: path.display().to_string(),
                source: SocketSource::Configured,
                added_at: None,
                healthy: path.exists(),
                last_health_check: None,
                key_count: None,
                order,
            });
            order += 1;
        }

        result
    }

    /// Update health status for a socket
    pub fn update_socket_health(
        &mut self,
        path: &PathBuf,
        healthy: bool,
        key_count: Option<usize>,
    ) {
        if let Some(socket) = self.watched_sockets.get_mut(path) {
            socket.last_healthy = Some(healthy);
            socket.last_health_check = Some(SystemTime::now());
            socket.key_count = key_count;
        }
        self.last_health_check = Some(SystemTime::now());
    }

    /// Get last health check time
    pub fn last_health_check(&self) -> Option<SystemTime> {
        self.last_health_check
    }

    /// Add a watched socket
    pub fn add_watched(&mut self, path: PathBuf) -> bool {
        if self.watched_sockets.contains_key(&path) {
            log::debug!("Socket already watched: {}", path.display());
            return false;
        }

        log::info!("Adding watched socket: {}", path.display());
        let log_path = path.clone();
        let socket = WatchedSocket::new(path.clone());
        self.watched_sockets.insert(path, socket);
        self.log_state(format!(
            "Active sockets after adding forwarded agent {}",
            log_path.display()
        ));
        true
    }

    /// Remove a watched socket
    pub fn remove_watched(&mut self, path: &PathBuf) -> bool {
        if let Some(_) = self.watched_sockets.remove(path) {
            log::info!("Removed watched socket: {}", path.display());
            self.log_state(format!(
                "Active sockets after removing forwarded agent {}",
                path.display()
            ));
            true
        } else {
            log::debug!("Socket not found in watched list: {}", path.display());
            false
        }
    }

    /// Validate all sockets and remove non-existent ones
    /// Returns list of removed socket paths
    pub fn validate_and_cleanup(&mut self) -> Vec<PathBuf> {
        let mut removed = Vec::new();

        // Check watched sockets
        self.watched_sockets.retain(|path, _| {
            if path.exists() {
                true
            } else {
                log::info!("Removing non-existent watched socket: {}", path.display());
                removed.push(path.clone());
                false
            }
        });

        if !removed.is_empty() {
            self.log_state("Active sockets after cleanup");
        }

        removed
    }

    /// Get count of watched sockets
    pub fn watched_count(&self) -> usize {
        self.watched_sockets.len()
    }

    /// Get count of configured sockets
    pub fn configured_count(&self) -> usize {
        self.configured_sockets.len()
    }

    /// Get total count of all sockets
    pub fn total_count(&self) -> usize {
        self.watched_count() + self.configured_count()
    }

    /// Check if a path is already being watched
    pub fn is_watched(&self, path: &PathBuf) -> bool {
        self.watched_sockets.contains_key(path)
    }

    /// Check if a path is in the configured list
    pub fn is_configured(&self, path: &PathBuf) -> bool {
        self.configured_sockets.contains(path)
    }

    /// Update the configured sockets list
    pub fn update_configured(&mut self, configured_sockets: Vec<PathBuf>) {
        self.configured_sockets = configured_sockets;
        self.log_state("Active sockets after configuration update");
    }

    /// Get the configured sockets list
    pub fn configured_sockets(&self) -> &[PathBuf] {
        &self.configured_sockets
    }

    /// Log the current socket ordering to aid debugging
    pub fn log_state(&self, context: impl AsRef<str>) {
        let context = context.as_ref();
        let ordered = self.get_ordered_sockets();
        if ordered.is_empty() {
            log::info!(
                "{}: no active agent sockets (watched: {}, configured: {})",
                context,
                self.watched_count(),
                self.configured_count()
            );
            return;
        }

        let ordered_paths = ordered
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");

        log::info!(
            "{}: {} active agent sockets (watched: {}, configured: {}); order: [{}]",
            context,
            ordered.len(),
            self.watched_count(),
            self.configured_count(),
            ordered_paths
        );
    }
}

/// Format a SystemTime as ISO 8601 string
fn format_system_time(time: SystemTime) -> String {
    let datetime: DateTime<Utc> = time.into();
    datetime.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_new_socket_manager() {
        let configured = vec![
            PathBuf::from("/tmp/agent1.sock"),
            PathBuf::from("/tmp/agent2.sock"),
        ];
        let manager = SocketManager::new(configured.clone());
        assert_eq!(manager.configured_count(), 2);
        assert_eq!(manager.watched_count(), 0);
        assert_eq!(manager.get_ordered_sockets(), configured);
    }

    #[test]
    fn test_add_watched_socket() {
        let configured = vec![PathBuf::from("/tmp/configured.sock")];
        let mut manager = SocketManager::new(configured);

        let watched = PathBuf::from("/tmp/watched.sock");
        assert!(manager.add_watched(watched.clone()));
        assert_eq!(manager.watched_count(), 1);
        assert!(manager.is_watched(&watched));

        // Adding same socket again should return false
        assert!(!manager.add_watched(watched));
        assert_eq!(manager.watched_count(), 1);
    }

    #[test]
    fn test_remove_watched_socket() {
        let mut manager = SocketManager::new(vec![]);
        let watched = PathBuf::from("/tmp/watched.sock");

        manager.add_watched(watched.clone());
        assert_eq!(manager.watched_count(), 1);

        assert!(manager.remove_watched(&watched));
        assert_eq!(manager.watched_count(), 0);

        // Removing non-existent socket should return false
        assert!(!manager.remove_watched(&watched));
    }

    #[test]
    fn test_ordering_watched_first() {
        let configured = vec![
            PathBuf::from("/tmp/configured1.sock"),
            PathBuf::from("/tmp/configured2.sock"),
        ];
        let mut manager = SocketManager::new(configured.clone());

        let watched1 = PathBuf::from("/tmp/watched1.sock");
        let watched2 = PathBuf::from("/tmp/watched2.sock");

        manager.add_watched(watched1.clone());
        thread::sleep(Duration::from_millis(10));
        manager.add_watched(watched2.clone());

        let ordered = manager.get_ordered_sockets();

        // Should be: watched2 (newest), watched1, configured1, configured2
        assert_eq!(ordered.len(), 4);
        assert_eq!(ordered[0], watched2);
        assert_eq!(ordered[1], watched1);
        assert_eq!(ordered[2], configured[0]);
        assert_eq!(ordered[3], configured[1]);
    }

    #[test]
    fn test_update_configured() {
        let initial = vec![PathBuf::from("/tmp/initial.sock")];
        let mut manager = SocketManager::new(initial);
        assert_eq!(manager.configured_count(), 1);

        let updated = vec![
            PathBuf::from("/tmp/updated1.sock"),
            PathBuf::from("/tmp/updated2.sock"),
        ];
        manager.update_configured(updated.clone());
        assert_eq!(manager.configured_count(), 2);
        assert_eq!(manager.get_ordered_sockets(), updated);
    }

    #[test]
    fn test_validate_and_cleanup_nonexistent() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().join("agent.sock");

        // Create a temporary file
        std::fs::File::create(&temp_path).unwrap();

        let mut manager = SocketManager::new(vec![]);
        manager.add_watched(temp_path.clone());
        assert_eq!(manager.watched_count(), 1);

        // File exists, should not be removed
        let removed = manager.validate_and_cleanup();
        assert_eq!(removed.len(), 0);
        assert_eq!(manager.watched_count(), 1);

        // Delete the file
        std::fs::remove_file(&temp_path).unwrap();

        // Now it should be removed
        let removed = manager.validate_and_cleanup();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], temp_path);
        assert_eq!(manager.watched_count(), 0);
    }

    #[test]
    fn test_uptime() {
        let manager = SocketManager::new(vec![]);
        // Uptime should be very small (< 1 second typically)
        assert!(manager.uptime_secs() < 2);
    }

    #[test]
    fn test_total_count() {
        let configured = vec![
            PathBuf::from("/tmp/c1.sock"),
            PathBuf::from("/tmp/c2.sock"),
        ];
        let mut manager = SocketManager::new(configured);
        assert_eq!(manager.total_count(), 2);

        manager.add_watched(PathBuf::from("/tmp/w1.sock"));
        assert_eq!(manager.total_count(), 3);
    }

    #[test]
    fn test_get_socket_info() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let configured_path = temp_dir.path().join("configured.sock");
        let watched_path = temp_dir.path().join("watched.sock");

        // Create the files so they appear healthy
        std::fs::File::create(&configured_path).unwrap();
        std::fs::File::create(&watched_path).unwrap();

        let mut manager = SocketManager::new(vec![configured_path.clone()]);
        manager.add_watched(watched_path.clone());

        let info = manager.get_socket_info();
        assert_eq!(info.len(), 2);

        // Watched should be first
        assert_eq!(info[0].source, SocketSource::Watched);
        assert_eq!(info[0].order, 1);
        assert!(info[0].added_at.is_some());
        assert!(info[0].healthy);

        // Configured should be second
        assert_eq!(info[1].source, SocketSource::Configured);
        assert_eq!(info[1].order, 2);
        assert!(info[1].added_at.is_none());
        assert!(info[1].healthy);
    }

    #[test]
    fn test_update_socket_health() {
        let mut manager = SocketManager::new(vec![]);
        let path = PathBuf::from("/tmp/test.sock");

        manager.add_watched(path.clone());

        // Initially no health check
        let info = manager.get_socket_info();
        assert!(info[0].last_health_check.is_none());
        assert!(info[0].key_count.is_none());

        // Update health
        manager.update_socket_health(&path, true, Some(3));

        let info = manager.get_socket_info();
        assert!(info[0].last_health_check.is_some());
        assert_eq!(info[0].key_count, Some(3));
        assert!(info[0].healthy);
    }

    #[test]
    fn test_is_configured() {
        let path = PathBuf::from("/tmp/test.sock");
        let manager = SocketManager::new(vec![path.clone()]);

        assert!(manager.is_configured(&path));
        assert!(!manager.is_configured(&PathBuf::from("/tmp/other.sock")));
    }
}

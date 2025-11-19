use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Manages both configured and watched sockets with proper ordering
#[derive(Debug, Clone)]
pub struct SocketManager {
    configured_sockets: Vec<PathBuf>,
    watched_sockets: HashMap<PathBuf, WatchedSocket>,
}

/// Represents a watched socket with metadata
#[derive(Debug, Clone)]
struct WatchedSocket {
    path: PathBuf,
    created_at: SystemTime,
}

impl SocketManager {
    /// Create a new SocketManager with configured sockets
    pub fn new(configured_sockets: Vec<PathBuf>) -> Self {
        let manager = Self {
            configured_sockets,
            watched_sockets: HashMap::new(),
        };
        manager.log_state("Initialized socket manager");
        manager
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

    /// Add a watched socket
    pub fn add_watched(&mut self, path: PathBuf) -> bool {
        if self.watched_sockets.contains_key(&path) {
            log::debug!("Socket already watched: {}", path.display());
            return false;
        }

        log::info!("Adding watched socket: {}", path.display());
        let log_path = path.clone();
        let socket = WatchedSocket {
            path: path.clone(),
            created_at: SystemTime::now(),
        };
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

    /// Check if a path is already being watched
    pub fn is_watched(&self, path: &PathBuf) -> bool {
        self.watched_sockets.contains_key(path)
    }

    /// Update the configured sockets list
    pub fn update_configured(&mut self, configured_sockets: Vec<PathBuf>) {
        self.configured_sockets = configured_sockets;
        self.log_state("Active sockets after configuration update");
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
}

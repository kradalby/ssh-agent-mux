use notify::{Event, EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, NoCache};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// Events emitted by the file watcher
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A new SSH forwarded agent socket was detected
    Added(PathBuf),
    /// An SSH forwarded agent socket was removed
    Removed(PathBuf),
}

/// Check if a path matches the SSH forwarded agent pattern
/// Pattern: /tmp/ssh-*/agent.*
pub fn is_ssh_forwarded_agent(path: &Path) -> bool {
    // Get the path as a string for pattern matching
    let path_str = match path.to_str() {
        Some(s) => s,
        None => return false,
    };

    // Check if path starts with /tmp/ssh-
    if !path_str.starts_with("/tmp/ssh-") {
        return false;
    }

    // Get the file name
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };

    // Check if file name starts with "agent."
    file_name.starts_with("agent.")
}

/// Start watching /tmp directory for SSH forwarded agents
/// Returns a receiver channel that will receive WatchEvent messages
pub async fn watch_tmp_directory(
    tx: mpsc::UnboundedSender<WatchEvent>,
) -> Result<Debouncer<notify::RecommendedWatcher, NoCache>, notify::Error> {
    let tmp_path = Path::new("/tmp");

    log::info!("Starting file watcher on /tmp for SSH forwarded agents");

    // Create debounced watcher (200ms debounce time)
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        None,
        move |result: DebounceEventResult| {
            match result {
                Ok(events) => {
                    for event in events {
                        handle_event(event.event, &tx);
                    }
                }
                Err(errors) => {
                    for error in errors {
                        log::error!("File watcher error: {:?}", error);
                    }
                }
            }
        },
    )?;

    // Watch /tmp directory recursively
    debouncer.watch(tmp_path, RecursiveMode::Recursive)?;

    log::info!("File watcher started successfully");

    Ok(debouncer)
}

/// Handle a file system event
fn handle_event(event: Event, tx: &mpsc::UnboundedSender<WatchEvent>) {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) => {
            for path in event.paths {
                if is_ssh_forwarded_agent(&path) && path.exists() {
                    log::debug!("Detected new SSH forwarded agent: {}", path.display());
                    if let Err(e) = tx.send(WatchEvent::Added(path.clone())) {
                        log::error!("Failed to send Added event for {}: {}", path.display(), e);
                    }
                }
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                if is_ssh_forwarded_agent(&path) {
                    log::debug!("Detected removed SSH forwarded agent: {}", path.display());
                    if let Err(e) = tx.send(WatchEvent::Removed(path.clone())) {
                        log::error!("Failed to send Removed event for {}: {}", path.display(), e);
                    }
                }
            }
        }
        _ => {
            // Ignore other event types
        }
    }
}

/// Scan /tmp directory for existing SSH forwarded agents
/// This should be called once at startup to detect any existing sockets
pub async fn scan_existing_agents() -> Result<Vec<PathBuf>, std::io::Error> {
    use tokio::fs;

    let mut agents = Vec::new();
    let tmp_path = Path::new("/tmp");

    log::debug!("Scanning /tmp for existing SSH forwarded agents");

    let mut entries = fs::read_dir(tmp_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Check if it's a directory matching ssh-*
        if path.is_dir() {
            if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                if dir_name.starts_with("ssh-") {
                    // Look for agent.* files in this directory
                    let mut agent_entries = fs::read_dir(&path).await?;
                    while let Some(agent_entry) = agent_entries.next_entry().await? {
                        let agent_path = agent_entry.path();
                        if is_ssh_forwarded_agent(&agent_path) && agent_path.exists() {
                            log::debug!("Found existing SSH forwarded agent: {}", agent_path.display());
                            agents.push(agent_path);
                        }
                    }
                }
            }
        }
    }

    log::info!("Found {} existing SSH forwarded agents", agents.len());
    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ssh_forwarded_agent_valid() {
        assert!(is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-kDBDw0c18X/agent.34640"
        )));
        assert!(is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-Pz1huKcZZO/agent.34737"
        )));
        assert!(is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-jSHs8H99CC/agent.34840"
        )));
    }

    #[test]
    fn test_is_ssh_forwarded_agent_invalid() {
        // Wrong directory
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/var/tmp/ssh-abc/agent.123"
        )));

        // Wrong prefix
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp/notsh-abc/agent.123")));

        // Wrong file name
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-abc/notAgent.123"
        )));
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp/ssh-abc/Agent.123")));

        // Missing agent prefix
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp/ssh-abc/123")));

        // Just the directory
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp/ssh-abc/")));
    }

    #[test]
    fn test_is_ssh_forwarded_agent_edge_cases() {
        // Empty path
        assert!(!is_ssh_forwarded_agent(Path::new("")));

        // Root
        assert!(!is_ssh_forwarded_agent(Path::new("/")));

        // /tmp itself
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp")));

        // Relative path (shouldn't match)
        assert!(!is_ssh_forwarded_agent(Path::new("ssh-abc/agent.123")));
    }

    #[tokio::test]
    async fn test_watch_event_types() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        // Test Added event
        tx.send(WatchEvent::Added(PathBuf::from("/tmp/ssh-test/agent.123")))
            .unwrap();
        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            WatchEvent::Added(PathBuf::from("/tmp/ssh-test/agent.123"))
        );

        // Test Removed event
        tx.send(WatchEvent::Removed(PathBuf::from(
            "/tmp/ssh-test/agent.123",
        )))
        .unwrap();
        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            WatchEvent::Removed(PathBuf::from("/tmp/ssh-test/agent.123"))
        );
    }

    #[tokio::test]
    async fn test_scan_existing_agents_empty_tmp() {
        // This test might fail in environments where /tmp has SSH agents
        // It's more of a smoke test to ensure the function doesn't panic
        match scan_existing_agents().await {
            Ok(agents) => {
                // Should succeed, might find 0 or more agents
                log::debug!("Found {} agents", agents.len());
                for agent in agents {
                    assert!(is_ssh_forwarded_agent(&agent));
                }
            }
            Err(e) => {
                // Might fail if /tmp doesn't exist or no permissions
                log::debug!("Scan failed (expected in some environments): {}", e);
            }
        }
    }
}

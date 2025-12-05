use notify::{Event, EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone, Copy)]
enum NamePattern {
    Prefix(&'static str),
    Exact(&'static str),
}

impl NamePattern {
    fn matches(&self, candidate: &str) -> bool {
        match self {
            NamePattern::Prefix(prefix) => candidate.starts_with(prefix),
            NamePattern::Exact(exact) => candidate == *exact,
        }
    }
}

#[derive(Clone, Copy)]
struct ForwardedAgentPattern {
    dir_pattern: NamePattern,
    file_pattern: NamePattern,
}

impl ForwardedAgentPattern {
    const fn new(dir_pattern: NamePattern, file_pattern: NamePattern) -> Self {
        Self {
            dir_pattern,
            file_pattern,
        }
    }
}

const FORWARDED_AGENT_PATTERNS: &[ForwardedAgentPattern] = &[
    ForwardedAgentPattern::new(NamePattern::Prefix("ssh-"), NamePattern::Prefix("agent.")),
    ForwardedAgentPattern::new(
        NamePattern::Prefix("auth-agent"),
        NamePattern::Exact("listener.sock"),
    ),
];

/// Events emitted by the file watcher
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A new SSH forwarded agent socket was detected
    Added(PathBuf),
    /// An SSH forwarded agent socket was removed
    Removed(PathBuf),
}

/// Check if a directory name matches SSH agent directory patterns
fn should_watch_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.starts_with("ssh-") || name.starts_with("auth-agent"))
        .unwrap_or(false)
}

/// Smart watcher that selectively watches directories in /tmp
/// to avoid permission errors on restricted directories
pub struct SmartWatcher {
    debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    watched_dirs: Arc<StdMutex<HashSet<PathBuf>>>,
}

impl SmartWatcher {
    /// Get list of currently watched directories
    pub fn watched_directories(&self) -> Vec<PathBuf> {
        self.watched_dirs
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }

    /// Try to add a directory to the watch list
    pub fn try_watch_directory(&mut self, path: &Path) -> bool {
        if !should_watch_directory(path) {
            return false;
        }

        let mut watched = self.watched_dirs.lock().unwrap();
        if watched.contains(path) {
            return true; // Already watching
        }

        match self.debouncer.watch(path, RecursiveMode::Recursive) {
            Ok(_) => {
                log::debug!("Now watching directory: {}", path.display());
                watched.insert(path.to_path_buf());
                true
            }
            Err(e) => {
                log::debug!("Cannot watch {}: {}", path.display(), e);
                false
            }
        }
    }

    /// Remove a directory from the watch list
    pub fn unwatch_directory(&mut self, path: &Path) {
        let mut watched = self.watched_dirs.lock().unwrap();
        if watched.remove(path) {
            if let Err(e) = self.debouncer.unwatch(path) {
                log::debug!("Error unwatching {}: {}", path.display(), e);
            }
        }
    }
}

/// Check if a path matches a forwarded SSH agent pattern
/// Supported patterns:
///   * /tmp/ssh-*/agent.*
///   * /tmp/auth-agent*/listener.sock
pub fn is_ssh_forwarded_agent(path: &Path) -> bool {
    if !path.starts_with(Path::new("/tmp")) {
        return false;
    }

    // Get parent directory name
    let parent_name = match path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    {
        Some(name) => name,
        None => return false,
    };

    // Get the file name
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };

    FORWARDED_AGENT_PATTERNS.iter().any(|pattern| {
        pattern.dir_pattern.matches(parent_name) && pattern.file_pattern.matches(file_name)
    })
}

/// Start watching /tmp directory for SSH forwarded agents
/// Returns a receiver channel that will receive WatchEvent messages
#[deprecated(note = "Use watch_tmp_directory_smart instead for better robustness")]
pub async fn watch_tmp_directory(
    tx: mpsc::UnboundedSender<WatchEvent>,
) -> Result<Debouncer<notify::RecommendedWatcher, RecommendedCache>, notify::Error> {
    let tmp_path = Path::new("/tmp");

    log::info!("Starting file watcher on /tmp for SSH forwarded agents");

    // Create debounced watcher (200ms debounce time)
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        None,
        move |result: DebounceEventResult| match result {
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
        },
    )?;

    // Watch /tmp directory recursively
    debouncer.watch(tmp_path, RecursiveMode::Recursive)?;

    log::info!("File watcher started successfully");

    Ok(debouncer)
}

/// Start smart watching of /tmp directory for SSH forwarded agents
///
/// This watches /tmp non-recursively, then selectively watches only
/// ssh-* and auth-agent* subdirectories to avoid permission errors
/// on restricted directories like /tmp/systemd-private-*.
pub async fn watch_tmp_directory_smart(
    tx: mpsc::UnboundedSender<WatchEvent>,
) -> Result<SmartWatcher, notify::Error> {
    let tmp_path = Path::new("/tmp");
    let watched_dirs = Arc::new(StdMutex::new(HashSet::new()));
    let watched_dirs_clone = watched_dirs.clone();
    let tx_clone = tx.clone();

    log::info!("Starting smart file watcher on /tmp for SSH forwarded agents");

    // Create debounced watcher (200ms debounce time)
    let debouncer = new_debouncer(
        Duration::from_millis(200),
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    handle_smart_event(event.event, &tx_clone, &watched_dirs_clone);
                }
            }
            Err(errors) => {
                for error in errors {
                    log::error!("File watcher error: {:?}", error);
                }
            }
        },
    )?;

    let mut watcher = SmartWatcher {
        debouncer,
        watched_dirs,
    };

    // Watch /tmp NON-recursively for new directory creation
    watcher
        .debouncer
        .watch(tmp_path, RecursiveMode::NonRecursive)?;

    // Selectively watch existing ssh-*/auth-agent* directories
    if let Ok(entries) = std::fs::read_dir(tmp_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && should_watch_directory(&path) {
                watcher.try_watch_directory(&path);
            }
        }
    }

    let watched_count = watcher.watched_dirs.lock().unwrap().len();
    log::info!(
        "Smart file watcher started, monitoring {} ssh/auth-agent directories",
        watched_count
    );

    Ok(watcher)
}

/// Handle events from the smart watcher
fn handle_smart_event(
    event: Event,
    tx: &mpsc::UnboundedSender<WatchEvent>,
    watched_dirs: &Arc<StdMutex<HashSet<PathBuf>>>,
) {
    let tmp_path = Path::new("/tmp");

    match event.kind {
        // Handle directory creation in /tmp - we may need to start watching it
        EventKind::Create(notify::event::CreateKind::Folder) => {
            for path in &event.paths {
                // Check if this is a new directory directly in /tmp
                if path.parent() == Some(tmp_path) && should_watch_directory(path) {
                    // We can't modify the debouncer from here (it's in the callback)
                    // but the scan_existing_agents() call will pick up new directories
                    // and we can trigger a manual re-scan via the control socket
                    log::info!(
                        "New SSH agent directory detected: {} (will be picked up on next scan)",
                        path.display()
                    );
                }
            }
        }

        // Handle socket creation/modification
        EventKind::Create(_) | EventKind::Modify(_) => {
            for path in &event.paths {
                if is_ssh_forwarded_agent(path) && path.exists() {
                    log::debug!("Detected new SSH forwarded agent: {}", path.display());
                    if let Err(e) = tx.send(WatchEvent::Added(path.clone())) {
                        log::error!("Failed to send Added event for {}: {}", path.display(), e);
                    }
                }
            }
        }

        // Handle removal
        EventKind::Remove(_) => {
            for path in &event.paths {
                // Check if an entire watched directory was removed
                if path.parent() == Some(tmp_path) {
                    let mut watched = watched_dirs.lock().unwrap();
                    if watched.remove(path) {
                        log::debug!("Watched directory removed: {}", path.display());
                    }
                }

                // Check if it's a socket being removed
                if is_ssh_forwarded_agent(path) {
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
                for pattern in FORWARDED_AGENT_PATTERNS {
                    if !pattern.dir_pattern.matches(dir_name) {
                        continue;
                    }

                    match pattern.file_pattern {
                        NamePattern::Exact(file_name) => {
                            let candidate = path.join(file_name);
                            if candidate.exists() {
                                log::debug!(
                                    "Found existing SSH forwarded agent: {}",
                                    candidate.display()
                                );
                                agents.push(candidate);
                            }
                        }
                        NamePattern::Prefix(prefix) => {
                            let mut agent_entries = fs::read_dir(&path).await?;
                            while let Some(agent_entry) = agent_entries.next_entry().await? {
                                let agent_path = agent_entry.path();
                                if let Some(entry_name) =
                                    agent_path.file_name().and_then(|n| n.to_str())
                                {
                                    if entry_name.starts_with(prefix)
                                        && agent_path.exists()
                                        && is_ssh_forwarded_agent(&agent_path)
                                    {
                                        log::debug!(
                                            "Found existing SSH forwarded agent: {}",
                                            agent_path.display()
                                        );
                                        agents.push(agent_path);
                                    }
                                }
                            }
                        }
                    }
                    // Continue checking other patterns, since multiple could match same directory
                }
            }
        }
    }

    log::info!("Found {} existing SSH forwarded agents", agents.len());
    Ok(agents)
}

/// Watcher mode - either smart file watching or polling fallback
pub enum WatchMode {
    /// Using smart file watcher (inotify/FSEvents)
    Smart(SmartWatcher),
    /// Using polling fallback (when file watching fails)
    Polling,
}

/// Result of attempting to start smart watching
pub struct WatchResult {
    /// The watcher mode that was started
    pub mode: WatchMode,
    /// Error message if fell back to polling
    pub fallback_reason: Option<String>,
}

/// Start watching with automatic fallback to polling
///
/// Tries to start the smart file watcher first. If that fails,
/// returns Polling mode instead with the error reason.
pub async fn start_watching(
    tx: mpsc::UnboundedSender<WatchEvent>,
) -> WatchResult {
    match watch_tmp_directory_smart(tx).await {
        Ok(watcher) => WatchResult {
            mode: WatchMode::Smart(watcher),
            fallback_reason: None,
        },
        Err(e) => {
            log::warn!(
                "Smart file watcher failed ({}), will use polling fallback",
                e
            );
            WatchResult {
                mode: WatchMode::Polling,
                fallback_reason: Some(e.to_string()),
            }
        }
    }
}

/// Run polling mode to detect changes to SSH forwarded agents
///
/// This is a fallback when file watching fails (e.g., due to permissions).
/// It periodically scans /tmp for SSH agent sockets and compares with
/// the known set.
pub async fn run_polling_loop(
    tx: mpsc::UnboundedSender<WatchEvent>,
    interval: Duration,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    use std::collections::HashSet;

    log::info!(
        "Starting polling fallback with {}s interval",
        interval.as_secs()
    );

    let mut known_agents: HashSet<PathBuf> = HashSet::new();

    // Initial scan
    if let Ok(agents) = scan_existing_agents().await {
        for agent in agents {
            known_agents.insert(agent);
        }
    }

    let mut ticker = tokio::time::interval(interval);
    // Skip the first tick (immediate)
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match scan_existing_agents().await {
                    Ok(current_agents) => {
                        let current_set: HashSet<PathBuf> = current_agents.into_iter().collect();

                        // Check for new agents
                        for agent in current_set.difference(&known_agents) {
                            log::debug!("Polling detected new agent: {}", agent.display());
                            if let Err(e) = tx.send(WatchEvent::Added(agent.clone())) {
                                log::error!("Failed to send Added event: {}", e);
                            }
                        }

                        // Check for removed agents
                        for agent in known_agents.difference(&current_set) {
                            log::debug!("Polling detected removed agent: {}", agent.display());
                            if let Err(e) = tx.send(WatchEvent::Removed(agent.clone())) {
                                log::error!("Failed to send Removed event: {}", e);
                            }
                        }

                        known_agents = current_set;
                    }
                    Err(e) => {
                        log::warn!("Polling scan failed: {}", e);
                    }
                }
            }
            _ = shutdown.recv() => {
                log::debug!("Polling loop received shutdown signal");
                break;
            }
        }
    }
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
        assert!(is_ssh_forwarded_agent(Path::new(
            "/tmp/auth-agent123456/listener.sock"
        )));
        assert!(is_ssh_forwarded_agent(Path::new(
            "/tmp/auth-agent9876543/listener.sock"
        )));
    }

    #[test]
    fn test_is_ssh_forwarded_agent_invalid() {
        // Wrong directory
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/var/tmp/ssh-abc/agent.123"
        )));

        // Wrong prefix
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/notsh-abc/agent.123"
        )));

        // Wrong file name
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-abc/notAgent.123"
        )));
        assert!(!is_ssh_forwarded_agent(Path::new("/tmp/ssh-abc/Agent.123")));
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/auth-agent1234/agent.1"
        )));
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/ssh-abc/listener.sock"
        )));
        assert!(!is_ssh_forwarded_agent(Path::new(
            "/tmp/auth-agent/listener2.sock"
        )));

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

    #[test]
    fn test_should_watch_directory() {
        // Should match ssh-* directories
        assert!(should_watch_directory(Path::new("/tmp/ssh-abc123")));
        assert!(should_watch_directory(Path::new("/tmp/ssh-XXXXXX")));

        // Should match auth-agent* directories
        assert!(should_watch_directory(Path::new("/tmp/auth-agent123456")));
        assert!(should_watch_directory(Path::new("/tmp/auth-agent999")));

        // Should NOT match other directories
        assert!(!should_watch_directory(Path::new("/tmp/systemd-private-abc")));
        assert!(!should_watch_directory(Path::new("/tmp/snap-private-tmp")));
        assert!(!should_watch_directory(Path::new("/tmp/random-dir")));
        assert!(!should_watch_directory(Path::new("/tmp/.X11-unix")));
    }
}

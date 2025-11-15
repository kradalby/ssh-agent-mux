use std::{
    ffi::OsString,
    fs,
    io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use harness::SshAgentInstance;

mod harness;
mod keys;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Helper to create a mock SSH forwarded agent socket structure
/// SSH creates these as /tmp/ssh-XXXXXX/agent.<ppid>
fn create_forwarded_agent_structure(
    base_dir: &Path,
    agent: &SshAgentInstance,
    suffix: &str,
) -> io::Result<PathBuf> {
    // Create ssh-XXXXXX directory (SSH uses random suffix with exactly 6 chars)
    // Pattern must match /tmp/ssh-* where * is typically 6 random chars
    let ssh_dir = base_dir.join(format!("ssh-{:06x}{}",
        (std::process::id() as u32).wrapping_add(suffix.len() as u32) % 0xFFFFFF,
        suffix.chars().take(2).collect::<String>()
    ));

    // Clean up if it already exists
    let _ = fs::remove_dir_all(&ssh_dir);
    fs::create_dir(&ssh_dir)?;

    // Create symlink to agent socket with the forwarded naming pattern
    let forwarded_path = ssh_dir.join(format!("agent.{}", std::process::id()));
    let agent_socket = PathBuf::from(format!("{}", agent.sock_path.display()));

    #[cfg(unix)]
    std::os::unix::fs::symlink(&agent_socket, &forwarded_path)?;

    Ok(forwarded_path)
}

/// Test that watcher detects a forwarded socket created after startup
#[test]
#[cfg(unix)]
fn detect_forwarded_socket_added() -> TestResult {
    // Use /tmp directly as that's where the watcher looks
    let tmp_dir = PathBuf::from("/tmp");

    // Start mux with watch enabled but no configured sockets
    let mux = SshAgentInstance::new_mux(
        "",
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    // Give the watcher time to start
    thread::sleep(Duration::from_millis(500));

    // Initially should have no keys
    let initial_keys = mux.list()?;
    assert!(initial_keys.is_empty(), "Should start with no keys");

    // Create a real agent with a key
    let forwarded_agent = SshAgentInstance::new_openssh()?;
    forwarded_agent.add(keys::TEST_KEY_ED25519)?;

    // Create forwarded socket structure pointing to this agent
    let forwarded_path = create_forwarded_agent_structure(&tmp_dir, &forwarded_agent, "-added")?;
    println!("Created forwarded socket at: {}", forwarded_path.display());

    // Wait for watcher to detect it (debounce is 200ms + processing time)
    thread::sleep(Duration::from_millis(800));

    // Now should have the key from the forwarded agent
    let keys_after = mux.list()?;
    println!("Keys after: {:?}", keys_after);

    // Cleanup first
    let ssh_dir = forwarded_path.parent().unwrap();
    let _ = fs::remove_file(&forwarded_path);
    let _ = fs::remove_dir(ssh_dir);

    assert!(
        !keys_after.is_empty(),
        "Should detect forwarded agent's keys"
    );
    assert!(
        keys_after.iter().any(|k| k.contains("integration-test-ed25519")),
        "Should have the ED25519 key from forwarded agent"
    );

    Ok(())
}

/// Test that watcher detects when a forwarded socket is removed
#[test]
#[cfg(unix)]
fn detect_forwarded_socket_removed() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    // Create a real agent with a key
    let forwarded_agent = SshAgentInstance::new_openssh()?;
    forwarded_agent.add(keys::TEST_KEY_ECDSA)?;

    // Create forwarded socket structure before starting mux
    let forwarded_path = create_forwarded_agent_structure(&tmp_dir, &forwarded_agent, "-removed")?;
    let ssh_dir = forwarded_path.parent().unwrap().to_path_buf();

    // Start mux - should detect existing forwarded socket during initial scan
    let mux = SshAgentInstance::new_mux(
        "",
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    // Give watcher time to scan existing sockets
    thread::sleep(Duration::from_millis(800));

    // Should have detected the existing forwarded socket
    let initial_keys = mux.list()?;
    println!("Initial keys: {:?}", initial_keys);

    // Remove the forwarded socket directory
    fs::remove_file(&forwarded_path)?;
    fs::remove_dir(&ssh_dir)?;

    // Wait for watcher to detect removal and validation to clean up
    thread::sleep(Duration::from_millis(800));

    // Keys should be gone (validation will remove non-existent sockets)
    let keys_after = mux.list()?;
    println!("Keys after removal: {:?}", keys_after);

    assert!(
        initial_keys.len() > keys_after.len() || keys_after.is_empty(),
        "Keys should be reduced or gone after socket removed. Before: {}, After: {}",
        initial_keys.len(),
        keys_after.len()
    );

    Ok(())
}

/// Test priority: forwarded sockets should come before configured sockets
#[test]
#[cfg(unix)]
fn forwarded_socket_priority() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    // Create a configured agent with RSA key
    let configured_agent = SshAgentInstance::new_openssh()?;
    configured_agent.add(keys::TEST_KEY_RSA)?;

    // Start mux with configured agent
    let config = format!(
        r##"agent_sock_paths = ["{}"]"##,
        configured_agent.sock_path.display()
    );

    let mux = SshAgentInstance::new_mux(
        &config,
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    thread::sleep(Duration::from_millis(500));

    // Should have RSA key from configured agent
    let keys_before = mux.list()?;
    assert_eq!(keys_before.len(), 1, "Should have 1 key initially");
    assert!(
        keys_before[0].contains("integration-test-rsa"),
        "Should have RSA key from configured agent"
    );

    // Create a forwarded agent with ED25519 key
    let forwarded_agent = SshAgentInstance::new_openssh()?;
    forwarded_agent.add(keys::TEST_KEY_ED25519)?;
    let forwarded_path = create_forwarded_agent_structure(&tmp_dir, &forwarded_agent, "-priority")?;

    thread::sleep(Duration::from_millis(800));

    // Should now have both keys, with ED25519 first (from forwarded agent)
    let keys_after = mux.list()?;
    println!("Keys with priority test: {:?}", keys_after);

    // Clean up
    let ssh_dir = forwarded_path.parent().unwrap();
    let _ = fs::remove_file(&forwarded_path);
    let _ = fs::remove_dir(ssh_dir);

    assert_eq!(keys_after.len(), 2, "Should have 2 keys after forwarding");

    // First key should be from forwarded agent (ED25519)
    assert!(
        keys_after[0].contains("integration-test-ed25519"),
        "Forwarded agent key should come first, got: {}",
        keys_after[0]
    );

    // Second key should be from configured agent (RSA)
    assert!(
        keys_after[1].contains("integration-test-rsa"),
        "Configured agent key should come second, got: {}",
        keys_after[1]
    );

    Ok(())
}

/// Test multiple forwarded sockets: newest should come first
#[test]
#[cfg(unix)]
fn multiple_forwarded_sockets_ordering() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    let mux = SshAgentInstance::new_mux(
        "",
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    thread::sleep(Duration::from_millis(500));

    // Create first forwarded agent with RSA key
    let first_agent = SshAgentInstance::new_openssh()?;
    first_agent.add(keys::TEST_KEY_RSA)?;
    let first_path = create_forwarded_agent_structure(&tmp_dir, &first_agent, "-multi1")?;

    thread::sleep(Duration::from_millis(800));

    // Should have RSA key
    let keys_after_first = mux.list()?;
    println!("Keys after first: {:?}", keys_after_first);
    assert_eq!(keys_after_first.len(), 1);
    assert!(keys_after_first[0].contains("integration-test-rsa"));

    // Create second forwarded agent with ECDSA key (newer)
    let second_agent = SshAgentInstance::new_openssh()?;
    second_agent.add(keys::TEST_KEY_ECDSA)?;
    let second_path = create_forwarded_agent_structure(&tmp_dir, &second_agent, "-multi2")?;

    thread::sleep(Duration::from_millis(800));

    // Should have both keys, with ECDSA first (newer socket)
    let keys = mux.list()?;
    println!("Keys with both agents: {:?}", keys);

    // Cleanup
    let _ = fs::remove_file(&first_path);
    let _ = fs::remove_dir(first_path.parent().unwrap());
    let _ = fs::remove_file(&second_path);
    let _ = fs::remove_dir(second_path.parent().unwrap());

    assert_eq!(keys.len(), 2, "Should have 2 keys from 2 forwarded agents");

    // Newer forwarded socket (ECDSA) should come first
    assert!(
        keys[0].contains("integration-test-ecdsa"),
        "Newer forwarded socket key should come first, got: {}",
        keys[0]
    );
    assert!(
        keys[1].contains("integration-test-rsa"),
        "Older forwarded socket key should come second, got: {}",
        keys[1]
    );

    Ok(())
}

/// Test that invalid sockets are properly cleaned up
#[test]
#[cfg(unix)]
fn cleanup_invalid_forwarded_sockets() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    // Create a real agent
    let forwarded_agent = SshAgentInstance::new_openssh()?;
    forwarded_agent.add(keys::TEST_KEY_ED25519)?;

    // Create forwarded socket structure
    let forwarded_path = create_forwarded_agent_structure(&tmp_dir, &forwarded_agent, "-cleanup")?;

    let mux = SshAgentInstance::new_mux(
        "",
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    thread::sleep(Duration::from_millis(800));

    // Should detect the socket
    let keys_before = mux.list()?;
    println!("Keys before cleanup: {:?}", keys_before);

    // Delete the forwarded symlink (simulating a disconnected agent)
    fs::remove_file(&forwarded_path)?;

    // Wait for validation to clean it up
    thread::sleep(Duration::from_millis(800));

    // Request keys again, which triggers validation
    let keys_after = mux.list()?;
    println!("Keys after cleanup: {:?}", keys_after);

    // Cleanup
    if let Some(parent) = forwarded_path.parent() {
        let _ = fs::remove_dir(parent);
    }

    assert!(
        keys_after.len() < keys_before.len() || keys_after.is_empty(),
        "Invalid socket should be cleaned up. Before: {}, After: {}",
        keys_before.len(),
        keys_after.len()
    );

    Ok(())
}

/// Test that watcher doesn't interfere with configured sockets
#[test]
#[cfg(unix)]
fn watcher_preserves_configured_sockets() -> TestResult {
    let configured = SshAgentInstance::new_openssh()?;
    configured.add(keys::TEST_KEY_RSA)?;

    let tmp_dir = PathBuf::from("/tmp");

    let config = format!(
        r##"agent_sock_paths = ["{}"]"##,
        configured.sock_path.display()
    );

    let mux = SshAgentInstance::new_mux(
        &config,
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    thread::sleep(Duration::from_millis(500));

    // Should have configured key
    let keys = mux.list()?;
    assert_eq!(keys.len(), 1);
    assert!(keys[0].contains("integration-test-rsa"));

    // Create and remove forwarded socket multiple times
    for i in 0..3 {
        let forwarded_agent = SshAgentInstance::new_openssh()?;
        forwarded_agent.add(keys::TEST_KEY_ED25519)?;
        let forwarded = create_forwarded_agent_structure(
            &tmp_dir,
            &forwarded_agent,
            &format!("-preserve{}", i),
        )?;
        thread::sleep(Duration::from_millis(500));
        fs::remove_file(&forwarded)?;
        if let Some(parent) = forwarded.parent() {
            let _ = fs::remove_dir(parent);
        }
        thread::sleep(Duration::from_millis(500));
        drop(forwarded_agent);
    }

    // Configured key should still be present
    let final_keys = mux.list()?;
    println!("Final keys: {:?}", final_keys);
    assert!(
        !final_keys.is_empty(),
        "Should still have configured socket"
    );
    assert!(
        final_keys.iter().any(|k| k.contains("integration-test-rsa")),
        "Configured socket should remain through forwarding changes"
    );

    Ok(())
}

/// Test that mux without watch flag doesn't detect forwarded sockets
#[test]
#[cfg(unix)]
fn no_detection_without_watch_flag() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    // Create forwarded agent before starting mux
    let forwarded_agent = SshAgentInstance::new_openssh()?;
    forwarded_agent.add(keys::TEST_KEY_ED25519)?;
    let forwarded_path = create_forwarded_agent_structure(&tmp_dir, &forwarded_agent, "-nowatch")?;

    // Start mux WITHOUT --watch-for-ssh-forward flag
    let mux = SshAgentInstance::new_mux("", None::<OsString>)?;

    thread::sleep(Duration::from_millis(500));

    // Should not detect forwarded socket without watch flag
    let keys = mux.list()?;

    // Cleanup
    let _ = fs::remove_file(&forwarded_path);
    if let Some(parent) = forwarded_path.parent() {
        let _ = fs::remove_dir(parent);
    }

    assert!(
        keys.is_empty(),
        "Should not detect forwarded sockets without --watch-for-ssh-forward"
    );

    Ok(())
}

/// Test debouncing: rapid events should be coalesced
#[test]
#[cfg(unix)]
fn debouncing_rapid_events() -> TestResult {
    let tmp_dir = PathBuf::from("/tmp");

    let mux = SshAgentInstance::new_mux(
        "",
        [OsString::from("--watch-for-ssh-forward")],
    )?;

    thread::sleep(Duration::from_millis(500));

    // Create and delete forwarded sockets rapidly
    for i in 0..3 {
        let agent = SshAgentInstance::new_openssh()?;
        agent.add(keys::TEST_KEY_ED25519)?;
        let socket = create_forwarded_agent_structure(&tmp_dir, &agent, &format!("-debounce{}", i))?;
        thread::sleep(Duration::from_millis(100));
        let _ = fs::remove_file(&socket);
        if let Some(parent) = socket.parent() {
            let _ = fs::remove_dir(parent);
        }
        thread::sleep(Duration::from_millis(100));
        drop(agent);
    }

    // Wait for debounce period
    thread::sleep(Duration::from_millis(500));

    // Should end with no keys (all sockets were removed)
    let keys = mux.list()?;
    println!("Keys after rapid events: {:?}", keys);
    assert!(
        keys.is_empty(),
        "Should handle rapid add/remove events correctly"
    );

    Ok(())
}

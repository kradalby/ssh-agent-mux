//! systemd notification support
//!
//! This module provides integration with systemd's service notification protocol,
//! enabling Type=notify services and watchdog functionality.

/// Notify systemd that the service is ready.
///
/// This should be called after all initialization is complete and the service
/// is ready to accept connections. When running under systemd with Type=notify,
/// the service is not considered started until this notification is sent.
#[cfg(feature = "systemd")]
pub fn notify_ready() {
    // Use false to keep NOTIFY_SOCKET for subsequent notifications (watchdog pings)
    match sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        Ok(()) => log::debug!("Sent READY notification to systemd"),
        Err(e) => log::debug!("Failed to notify systemd (not running under systemd?): {}", e),
    }
}

#[cfg(not(feature = "systemd"))]
pub fn notify_ready() {
    log::debug!("systemd notify support not compiled in");
}

/// Send a watchdog ping to systemd.
///
/// This should be called periodically (more frequently than WatchdogSec)
/// to indicate the service is still healthy. If systemd doesn't receive
/// a watchdog ping within the configured interval, it will restart the service.
#[cfg(feature = "systemd")]
pub fn notify_watchdog() {
    // Use false to keep NOTIFY_SOCKET for subsequent watchdog pings
    match sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]) {
        Ok(()) => log::debug!("Sent WATCHDOG ping to systemd"),
        Err(e) => log::warn!("Failed to send watchdog ping: {}", e),
    }
}

#[cfg(not(feature = "systemd"))]
pub fn notify_watchdog() {
    // No-op when systemd support is not compiled in
}

/// Notify systemd of the service status message.
///
/// This updates the status line shown by `systemctl status`.
#[cfg(feature = "systemd")]
pub fn notify_status(status: &str) {
    // Use false to keep NOTIFY_SOCKET for subsequent notifications
    match sd_notify::notify(false, &[sd_notify::NotifyState::Status(status)]) {
        Ok(()) => log::trace!("Updated systemd status: {}", status),
        Err(e) => log::trace!("Failed to update status: {}", e),
    }
}

#[cfg(not(feature = "systemd"))]
pub fn notify_status(_status: &str) {
    // No-op when systemd support is not compiled in
}

/// Check if we're running under systemd with watchdog enabled.
///
/// Returns the watchdog interval in microseconds if enabled, None otherwise.
#[cfg(feature = "systemd")]
pub fn watchdog_enabled() -> Option<u64> {
    let mut usec = 0u64;
    // Use false to keep WATCHDOG_USEC/WATCHDOG_PID in environment (though not strictly needed)
    if sd_notify::watchdog_enabled(false, &mut usec) {
        Some(usec)
    } else {
        None
    }
}

#[cfg(not(feature = "systemd"))]
pub fn watchdog_enabled() -> Option<u64> {
    None
}

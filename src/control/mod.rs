//! Control interface for ssh-agent-mux daemon.
//!
//! This module provides:
//! - Protocol types for client-daemon communication
//! - Control server that listens on a Unix socket
//! - Control client for CLI commands
//!
//! The control socket uses a JSON-over-Unix-socket protocol with newline-delimited messages.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{default_control_path, ControlClient, ControlClientError};
pub use protocol::*;
pub use server::{ControlServer, ControlServerState, SelfDeletingControlSocket};

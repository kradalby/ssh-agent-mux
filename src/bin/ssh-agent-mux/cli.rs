use std::{env, fs::File, io::Read, path::PathBuf};

use clap_serde_derive::{
    clap::{self, Parser, Subcommand, ValueEnum},
    serde::{self, Deserialize, Serialize},
    ClapSerde,
};
use color_eyre::eyre::Result as EyreResult;
use expand_tilde::ExpandTilde;
use log::LevelFilter;

use crate::service;

const APP_VERSION: &str = env!("SSH_AGENT_MUX_BUILD_VERSION");

fn default_config_path() -> PathBuf {
    let config_dir = env::var_os("XDG_CONFIG_HOME")
        .or_else(|| Some("~/.config".into()))
        .map(PathBuf::from)
        .and_then(|p| p.expand_tilde_owned().ok())
        .expect("HOME not defined in environment");

    config_dir
        .join(env!("CARGO_PKG_NAME"))
        .join(concat!(env!("CARGO_PKG_NAME"), ".toml"))
}

fn default_listen_path() -> PathBuf {
    PathBuf::from(concat!("~/.ssh/", env!("CARGO_PKG_NAME"), ".sock"))
}

/// Derive control socket path from listen path
pub fn derive_control_path(listen_path: &PathBuf) -> PathBuf {
    ssh_agent_mux::control::default_control_path(listen_path)
}

/// Get the default control socket path for client commands
pub fn default_control_socket() -> PathBuf {
    derive_control_path(&default_listen_path().expand_tilde_owned().unwrap_or_default())
}

#[derive(Parser)]
#[command(author, version = APP_VERSION, about)]
pub struct Args {
    /// Control socket path (for client commands)
    #[arg(long = "control-socket", short = 's', global = true)]
    pub control_socket: Option<PathBuf>,

    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    /// Subcommand to run
    #[command(subcommand)]
    pub command: Command,
}

/// CLI commands
#[derive(Subcommand)]
pub enum Command {
    /// Run the daemon (serve mode)
    Serve {
        /// Config file
        #[arg(short, long = "config", default_value_os_t = default_config_path())]
        config_path: PathBuf,

        /// Config from file or args
        #[command(flatten)]
        config: <Config as ClapSerde>::Opt,
    },

    /// Show daemon status
    Status,

    /// List upstream agent sockets
    List,

    /// List all available SSH keys
    ListKeys,

    /// Re-scan for forwarded agents
    Reload,

    /// Check socket health, remove stale sockets
    Validate,

    /// Add a socket to the watched list
    Add {
        /// Path to the socket to add
        path: PathBuf,
    },

    /// Remove a socket from the watched list
    Remove {
        /// Path to the socket to remove
        path: PathBuf,
    },

    /// Full health check of all sockets
    Health,
}


#[derive(ClapSerde, Clone, Serialize)]
pub struct Config {
    /// Listen path
    #[default(default_listen_path())]
    #[arg(short, long = "listen")]
    pub listen_path: PathBuf,

    /// Control socket path (from config file)
    #[arg(skip)]
    pub control_socket_path: Option<PathBuf>,

    /// Log level for agent
    #[default(LogLevel::Warn)]
    #[arg(long, value_enum)]
    pub log_level: LogLevel,

    /// Optional log file for agent (logs to standard output, otherwise)
    #[arg(long, num_args = 1)]
    pub log_file: Option<PathBuf>,

    /// Agent sockets to multiplex
    #[arg()]
    pub agent_sock_paths: Vec<PathBuf>,

    /// Watch /tmp for SSH forwarded agents
    #[default(false)]
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub watch_for_ssh_forward: bool,

    /// Health check interval in seconds (0 to disable)
    #[default(60u64)]
    #[arg(long)]
    pub health_check_interval: u64,

    // Following are part of command line args, but
    // not in configuration file
    /// Config file path (not an arg; copied from struct Args)
    #[arg(skip)]
    #[serde(skip_deserializing, skip_serializing)]
    pub config_path: PathBuf,

    #[serde(skip_deserializing, skip_serializing)]
    #[command(flatten)]
    pub service: service::ServiceArgs,
}

impl Config {
    pub fn from_serve_args(config_path: PathBuf, mut config_opt: <Config as ClapSerde>::Opt) -> EyreResult<Self> {
        let mut config = if let Ok(mut f) = File::open(&config_path) {
            log::info!("Read configuration from {}", config_path.display());
            let mut config_text = String::new();
            f.read_to_string(&mut config_text)?;
            let file_config = toml::from_str::<<Config as ClapSerde>::Opt>(&config_text)?;
            Config::from(file_config).merge(&mut config_opt)
        } else {
            Config::from(&mut config_opt)
        };

        config.config_path = config_path;
        config.listen_path = config.listen_path.expand_tilde_owned()?;
        config.log_file = config
            .log_file
            .map(|p| p.expand_tilde_owned())
            .transpose()?;
        config.agent_sock_paths = config
            .agent_sock_paths
            .into_iter()
            .map(|p| p.expand_tilde_owned())
            .collect::<Result<_, _>>()?;

        // Expand control socket path if set in config
        if let Some(ref path) = config.control_socket_path {
            config.control_socket_path = Some(path.expand_tilde_owned()?);
        }

        Ok(config)
    }

    /// Get the control socket path, deriving from listen_path if not set
    pub fn get_control_socket_path(&self) -> PathBuf {
        self.control_socket_path
            .clone()
            .unwrap_or_else(|| derive_control_path(&self.listen_path))
    }
}

#[derive(ValueEnum, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    #[value(hide = true)]
    Trace = 5,
}

impl From<LogLevel> for LevelFilter {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Error => LevelFilter::Error,
            LogLevel::Warn => LevelFilter::Warn,
            LogLevel::Info => LevelFilter::Info,
            LogLevel::Debug => LevelFilter::Debug,
            LogLevel::Trace => LevelFilter::Trace,
        }
    }
}

impl Args {
    pub fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }

    /// Get the control socket path for client commands
    pub fn get_control_socket(&self) -> PathBuf {
        self.control_socket
            .clone()
            .map(|p| p.expand_tilde_owned().unwrap_or(p))
            .unwrap_or_else(default_control_socket)
    }
}

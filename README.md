# `ssh-agent-mux` - Combine keys from multiple SSH agents into a single agent socket

Numerous types of SSH agents exist, such as the [1Password SSH agent](https://developer.1password.com/docs/ssh/agent/), which allows access to private keys in shared vaults, or [yubikey-agent](https://github.com/FiloSottile/yubikey-agent), allowing seamless access to private keys stored on [YubiKey](https://www.yubico.com/products/) cryptography devices. The `ssh` command allows using only one agent at-a-time, requiring you to configure per-server [`IdentityAgent`](https://www.mankier.com/5/ssh_config#IdentityAgent) settings or change the `SSH_AUTH_SOCK` environment variable depending on which agent you wish to use.

`ssh-agent-mux` combines multiple agents' keys into a single agent, allowing you to configure an SSH client just once. Provide all "upstream" SSH agents' `SSH_AUTH_SOCK` paths in the `ssh-agent-mux` [configuration](#configuration) and [run](#usage) `ssh-agent-mux` via your login scripts or OS's user service manager. Point your SSH configuration at `ssh-agent-mux`'s socket, and it will offer all available public keys from upstream agents as available for authentication.

## Features

* Simple TOML configuration syntax
* [systemd](https://systemd.io/) and [launchd](https://en.wikipedia.org/wiki/Launchd) user service manager integration
* [`session-bind@openssh.com` extension](https://github.com/openssh/openssh-portable/blob/46e52fdae08b89264a0b23f94391c2bf637def34/PROTOCOL.agent) pass-through support for agents that support key usage constraints
* **Control interface** - Inspect and manage the running daemon via CLI commands
* **SSH agent forwarding detection** - Automatically detect and use forwarded agents (`ssh -A`)
* **Health checking** - Periodic validation of upstream agent sockets with automatic cleanup

Go ahead and [submit an issue](https://github.com/overhacked/ssh-agent-mux/issues/new) if there's something that would make `ssh-agent-mux` more useful to you or if it isn't working as it should!

## Installation

### From crates.io

`ssh-agent-mux` can be installed from [crates.io](https://crates.io/crates/ssh-agent-mux):

```console
$ cargo install ssh-agent-mux
```

The minimum supported Rust version is `1.75.0`.

### Binary releases

Download binaries for various operating systems and architectures from the [releases page](https://github.com/overhacked/ssh-agent-mux/releases).

### Build from source

1. Clone the repository:
   ```console
   $ git clone https://github.com/overhacked/ssh-agent-mux.git && cd ssh-agent-mux/
   ```
2. Build:
   ```console
   $ cargo build --release
   ```

   The resulting binary is located at `target/release/ssh-agent-mux`
3. (Optional) Copy the binary to another location on your machine:
   ```console
   $ mkdir -p ~/bin && cp target/release/ssh-agent-mux ~/bin/
   ```

## Usage

### Linux (systemd)

```console
$ ssh-agent-mux --install-service

$ ssh-agent-mux --restart-service
OR
$ systemctl --user enable --now ssh-agent-mux.service
```

### macOS
```console
$ ssh-agent-mux --install-service
```

Service will automatically start as soon as it is installed.

## Configuration

`ssh-agent-mux` configuration is in [TOML](https://toml.io/en/v1.0.0) format. The default configuration file location is `~/.config/ssh-agent-mux/ssh-agent-mux.toml`. A simple configuration might look like:

```toml
agent_sock_paths = [
	"~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock",
	"~/Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh",
	"~/.ssh/yubikey-agent.sock",
]
```

The order of `agent_sock_paths` affects the order in which public keys are offered to an SSH server. If keys from multiple agents are listed on the server in your `authorized_keys` file, the agent listed first will be the one selected to authenticate with the server.

You can also specify all configuration on the command line, without using a configuration file at all. Any options specified on the command line override configuration file settings. To see the format of command line options, run:

```console
$ ssh-agent-mux --help
```

### Configuration file options

#### `agent_sock_paths` *[Array](https://toml.io/en/v1.0.0#array)*

Socket paths of upstream SSH agents to combine keys from. Must be specified as absolute paths. The order of `agent_sock_paths` affects the order in which public keys are offered to an SSH server. If keys from multiple agents are listed on the server in your `authorized_keys` file, the agent listed first will be the one selected to authenticate with the server.

#### `listen_path` *[String](https://toml.io/en/v1.0.0#string)*

`ssh-agent-mux`'s own socket path. Your SSH client's agent socket (usually the `SSH_AUTH_SOCK` environment variable or the `IdentityAgent` configuration setting) must be set to this path.

*Default*: `~/.ssh/ssh-agent-mux.sock`

#### `log_level` *[String](https://toml.io/en/v1.0.0#string)*

Controls the verbosity of `ssh-agent-mux`'s output. Valid values are: `error`, `warn`, `info`, and `debug`. For development and debugging, the [`RUST_LOG` environment variable](https://docs.rs/env_logger/latest/env_logger/#enabling-logging) is also supported and overrides any `log_level` setting.

*Default*: `warn`

#### `watch_for_ssh_forward` *[Boolean](https://toml.io/en/v1.0.0#boolean)*

Enable automatic detection of SSH forwarded agents. When enabled, `ssh-agent-mux` watches `/tmp` for SSH agent sockets that are forwarded via `ssh -A`. It recognizes both traditional OpenSSH sockets (`/tmp/ssh-*/agent.*`) and systemd/gnome-keyring style sockets (`/tmp/auth-agent*/listener.sock`). Detected forwarded agents are automatically added and their keys are offered with higher priority than configured agents.

This is useful when SSH-ing into a remote machine and then SSH-ing from that machine to other systems - the forwarded agent will be automatically detected and used.

*Default*: `false`

> **Note:** The watcher must see the real system `/tmp`. When running `ssh-agent-mux` as a systemd service (including the provided NixOS/Home Manager modules), make sure the service is not using `PrivateTmp`. The default `--install-service` configuration already leaves `PrivateTmp` disabled; the Nix modules automatically disable it when `watchForSSHForward = true`.

**Example with SSH forwarding detection enabled:**

```toml
# Static agents (e.g., local 1Password, Secretive)
agent_sock_paths = [
	"~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock",
	"~/Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh",
]

# Enable SSH forwarding detection
watch_for_ssh_forward = true
```

**Key Priority:**
1. Forwarded agents (newest first) - automatically detected when `watch_for_ssh_forward = true`
2. Configured agents (in order) - from `agent_sock_paths`

This means when you SSH into a machine with `ssh -A`, the forwarded agent's keys will be tried first, falling back to local agents if authentication fails.

#### `health_check_interval` *[Integer](https://toml.io/en/v1.0.0#integer)*

Interval in seconds between health checks of upstream agent sockets. Stale sockets (from disconnected SSH sessions or stopped agents) are automatically removed.

*Default*: `60`

Set to `0` to disable periodic health checks.

#### `control_socket_path` *[String](https://toml.io/en/v1.0.0#string)*

Path for the control socket used by CLI commands. If not set, defaults to the listen path with `.ctl` extension instead of `.sock`.

*Default*: Derived from `listen_path` (e.g., `~/.ssh/ssh-agent-mux.ctl`)

## CLI Commands

`ssh-agent-mux` provides CLI commands to inspect and manage the running daemon. These commands communicate with the daemon via the control socket.

### Available Commands

| Command | Description |
|---------|-------------|
| `status` | Show daemon status (uptime, version, socket count) |
| `list` | List upstream agent sockets with health status |
| `list-keys` | List all available SSH keys with fingerprints |
| `reload` | Re-scan for forwarded agents |
| `validate` | Check socket health and remove stale sockets |
| `add <path>` | Add a socket to the watched list |
| `remove <path>` | Remove a socket from the watched list |
| `health` | Full health check of all sockets |

### Command Options

* `--control-socket <path>` - Override control socket path
* `--json` - Output in JSON format (for scripting)

### Examples

```console
$ ssh-agent-mux status
ssh-agent-mux v0.2.0 (abc1234)
  PID:            412623
  Uptime:         2h 34m 12s

Sockets:
  Agent:          ~/.ssh/ssh-agent-mux.sock
  Control:        ~/.ssh/ssh-agent-mux.ctl

Watch:
  Enabled:        yes
  Status:         active

Stats:
  Upstream:       2 socket(s)
  Keys:           3 available
```

```console
$ ssh-agent-mux list
ORDER  SOURCE      HEALTHY  ADDED                 PATH
1      watched     yes      2024-12-05 13:28:10   /tmp/ssh-abc123/agent.12345
2      configured  yes      -                     ~/.1password/agent.sock
```

```console
$ ssh-agent-mux list-keys
FINGERPRINT                                        TYPE      COMMENT                    SOURCE
SHA256:3gkj/C+JPL9zkcOAjdo14kCe2S14qrw...          ed25519   user@laptop                /tmp/ssh-abc123/agent.12345
SHA256:Abc123...                                   rsa       backup-key                 ~/.1password/agent.sock
```

```console
$ ssh-agent-mux --json status
{
  "version": "0.2.0",
  "git_commit": "abc1234",
  "uptime_secs": 9252,
  "pid": 412623,
  "listening_on": "/home/user/.ssh/ssh-agent-mux.sock",
  "control_socket": "/home/user/.ssh/ssh-agent-mux.ctl",
  "watch_enabled": true,
  "watcher_status": "active",
  "socket_count": 2,
  "key_count": 3
}
```

### Troubleshooting

**"Failed to connect to daemon"**

The daemon may not be running, or the control socket path is incorrect. Check:
1. Is `ssh-agent-mux` running? (`systemctl --user status ssh-agent-mux`)
2. Use `--control-socket` to specify the correct path

**Watcher showing "polling_fallback"**

The file watcher couldn't monitor `/tmp` directly (often due to permission restrictions on `/tmp/systemd-private-*` directories). The daemon has automatically fallen back to periodic polling, which is slightly less responsive but still functional.

## Related projects

* [`ssh-manager`](https://github.com/omegion/ssh-manager): key manager for 1Password, Bitwarden, and AWS S3
* [`OmniSSHAgent`](https://github.com/masahide/OmniSSHAgent?tab=readme-ov-file): unifies multiple communication methods for SSH agents on Windows
* [`ssh-ident`](https://github.com/ccontavalli/ssh-ident): load ssh-agent identities on demand
* [`sshecret`](https://github.com/thcipriani/sshecret): "wrapper around ssh that automatically manages multiple `ssh-agent`s, each containing only a single ssh key"
* [`sshield`](https://github.com/gotlougit/sshield): drop-in ssh-agent replacement written in Rust using `russh`

## License

Dual-licensed under either [Apache License Version 2.0](https://opensource.org/license/apache-2-0) or [BSD 3-clause License](https://opensource.org/license/bsd-3-clause). You can choose between either one of them if you use this work.

`SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause`

## Copyright

Copyright &copy; 2024-2025, [Ross Williams](mailto:ross@ross-williams.net)

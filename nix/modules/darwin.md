# SSH Agent Mux - nix-darwin Module

This module provides a launchd user service for running ssh-agent-mux on macOS via nix-darwin.

## Usage

Import the module in your nix-darwin configuration:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    darwin.url = "github:LnL7/nix-darwin";
    darwin.inputs.nixpkgs.follows = "nixpkgs";
    
    ssh-agent-mux.url = "github:overhacked/ssh-agent-mux";
    ssh-agent-mux.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, darwin, ssh-agent-mux, ... }: {
    darwinConfigurations.myMac = darwin.lib.darwinSystem {
      system = "aarch64-darwin";
      modules = [
        ssh-agent-mux.darwinModules.default
        {
          services.ssh-agent-mux = {
            enable = true;
            agentSockets = [
              "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
              "~/.ssh/yubikey-agent.sock"
            ];
            logLevel = "info";
          };
        }
      ];
    };
  };
}
```

## Options

### `services.ssh-agent-mux.enable`

**Type:** boolean

**Default:** `false`

Enable the SSH Agent Mux service.

### `services.ssh-agent-mux.agentSockets`

**Type:** list of strings

**Default:** `[]`

List of agent socket paths to multiplex. Order matters - the first socket's keys will be offered to SSH servers first.

**Example:**
```nix
agentSockets = [
  "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
  "~/Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh"
  "~/.ssh/yubikey-agent.sock"
];
```

### `services.ssh-agent-mux.listenPath`

**Type:** string

**Default:** `"~/.ssh/ssh-agent-mux.sock"`

Path where ssh-agent-mux will create its multiplexed socket.

### `services.ssh-agent-mux.watchForSSHForward`

**Type:** boolean

**Default:** `false`

Enable automatic detection of SSH forwarded agents (via `ssh -A`).

When enabled, ssh-agent-mux will watch for SSH agent sockets forwarded from remote machines and automatically include them. Forwarded agents are prioritized (newest first) before configured agents.

### `services.ssh-agent-mux.logLevel`

**Type:** one of "error", "warn", "info", "debug"

**Default:** `"info"`

Log level for ssh-agent-mux.

### `services.ssh-agent-mux.package`

**Type:** package

**Default:** `pkgs.ssh-agent-mux`

The ssh-agent-mux package to use.

### `services.ssh-agent-mux.socketPath` (read-only)

**Type:** string

The resolved absolute path to the multiplexed socket. Use this in your shell configuration:

```nix
home.sessionVariables.SSH_AUTH_SOCK = config.services.ssh-agent-mux.socketPath;
```

## Service Management

The module creates a launchd user agent at:
```
~/Library/LaunchAgents/org.nixos.ssh-agent-mux.plist
```

### Managing the Service

After activating your nix-darwin configuration, the service will start automatically.

**Check service status:**
```bash
launchctl list | grep ssh-agent-mux
```

**View logs:**
```bash
tail -f ~/Library/Logs/ssh-agent-mux.log
tail -f ~/Library/Logs/ssh-agent-mux.error.log
```

**Manually stop the service:**
```bash
launchctl stop org.nixos.ssh-agent-mux
```

**Manually start the service:**
```bash
launchctl start org.nixos.ssh-agent-mux
```

## Environment Variables

The module automatically sets `SSH_AUTH_SOCK` system-wide to point to the multiplexed socket.

Alternatively, you can set it manually in your shell configuration:

```bash
# In ~/.zshrc or ~/.bashrc
export SSH_AUTH_SOCK="${HOME}/.ssh/ssh-agent-mux.sock"
```

Or use the `socketPath` option in home-manager:

```nix
home.sessionVariables.SSH_AUTH_SOCK = config.services.ssh-agent-mux.socketPath;
```

## Example Configurations

### Basic Configuration with 1Password

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
  ];
};
```

### Multiple Agents

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
    "~/Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh"
    "~/.ssh/yubikey-agent.sock"
  ];
  logLevel = "debug";
};
```

### With SSH Forwarding Detection

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
  ];
  watchForSSHForward = true;
  logLevel = "info";
};
```

## Troubleshooting

### Service won't start

Check the error log:
```bash
cat ~/Library/Logs/ssh-agent-mux.error.log
```

### SSH authentication not working

Verify the socket path:
```bash
echo $SSH_AUTH_SOCK
ls -la $SSH_AUTH_SOCK
```

List available keys:
```bash
ssh-add -L
```

### No keys visible

Ensure the upstream agent sockets exist:
```bash
ls -la ~/Library/Group\ Containers/2BUA8C4S2C.com.1password/t/agent.sock
```

Check if upstream agents have keys:
```bash
SSH_AUTH_SOCK=~/Library/Group\ Containers/2BUA8C4S2C.com.1password/t/agent.sock ssh-add -L
```

## See Also

- [ssh-agent-mux README](https://github.com/overhacked/ssh-agent-mux)
- [nix-darwin documentation](https://github.com/LnL7/nix-darwin)
- [1Password SSH Agent](https://developer.1password.com/docs/ssh/)
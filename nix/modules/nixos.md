# SSH Agent Mux - NixOS Module

This module provides a systemd *user* service for running ssh-agent-mux directly from your NixOS system configuration.

## Usage

Import the module in your NixOS configuration and enable it:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    ssh-agent-mux.url = "github:overhacked/ssh-agent-mux";
    ssh-agent-mux.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, ssh-agent-mux, ... }: {
    nixosConfigurations.myHost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ssh-agent-mux.nixosModules.default
        {
          services.ssh-agent-mux = {
            enable = true;
            agentSockets = [
              "~/.1password/agent.sock"
              "~/.ssh/yubikey-agent.sock"
            ];
            watchForSSHForward = true;
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

Enable the SSH Agent Mux systemd user service.

### `services.ssh-agent-mux.agentSockets`

**Type:** list of strings  
**Default:** `[]`

List of agent socket paths to multiplex. Order matters – sockets listed first have their keys offered first. `~` is expanded to the user's home directory.

**Example:**

```nix
agentSockets = [
  "~/.1password/agent.sock"
  "~/.ssh/yubikey-agent.sock"
];
```

### `services.ssh-agent-mux.listenPath`

**Type:** string  
**Default:** `"~/.ssh/ssh-agent-mux.sock"`

Path where ssh-agent-mux will create its multiplexed socket. Supports paths relative to the user's home directory.

### `services.ssh-agent-mux.watchForSSHForward`

**Type:** boolean  
**Default:** `false`

Enable automatic detection of SSH agent sockets forwarded via `ssh -A`. Forwarded agents are prioritized ahead of statically configured sockets.

### `services.ssh-agent-mux.logLevel`

**Type:** one of `"error"`, `"warn"`, `"info"`, `"debug"`  
**Default:** `"info"`

Log level for ssh-agent-mux.

### `services.ssh-agent-mux.package`

**Type:** package  
**Default:** `pkgs.ssh-agent-mux`

Package providing the `ssh-agent-mux` binary.

### `services.ssh-agent-mux.socketPath` (read-only)

**Type:** string

Resolved socket path that you can reference in shell configuration. `~` is automatically converted to `$HOME`, making it safe to assign directly to `SSH_AUTH_SOCK`.

## Service Management

The module installs a systemd *user* service named `ssh-agent-mux.service`. It is automatically started for every user session once enabled.

### Managing the Service

```bash
# Check status
systemctl --user status ssh-agent-mux

# Restart
systemctl --user restart ssh-agent-mux

# Start/stop manually
systemctl --user start ssh-agent-mux
systemctl --user stop ssh-agent-mux

# View logs
journalctl --user -u ssh-agent-mux -f
```

## Environment Variables

The module sets `SSH_AUTH_SOCK` via `environment.sessionVariables`, pointing it at the configured socket. You may need to reload your shell or log out/log back in for this to take effect.

```bash
echo "$SSH_AUTH_SOCK"
# → /home/you/.ssh/ssh-agent-mux.sock
```

If you prefer to manage the variable manually (for instance, in a per-user shell config), use the read-only `services.ssh-agent-mux.socketPath` helper.

```nix
programs.zsh.initExtra = ''
  export SSH_AUTH_SOCK=${config.services.ssh-agent-mux.socketPath}
'';
```

## Example Configurations

### Basic Configuration

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/.1password/agent.sock"
  ];
};
```

### Multiple Agents with Debug Logging

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/.1password/agent.sock"
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
    "~/.ssh/yubikey-agent.sock"
  ];
  watchForSSHForward = true;
};
```

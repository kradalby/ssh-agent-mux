# SSH Agent Mux - home-manager Module

This module provides a systemd user service for running ssh-agent-mux on Linux/NixOS via home-manager.

## Usage

Import the module in your home-manager configuration:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    
    ssh-agent-mux.url = "github:overhacked/ssh-agent-mux";
    ssh-agent-mux.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, home-manager, ssh-agent-mux, ... }: {
    homeConfigurations.myuser = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        ssh-agent-mux.homeManagerModules.default
        {
          services.ssh-agent-mux = {
            enable = true;
            agentSockets = [
              "~/.1password/agent.sock"
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
  "~/.1password/agent.sock"
  "~/.ssh/yubikey-agent.sock"
];
```

### `services.ssh-agent-mux.listenPath`

**Type:** string

**Default:** `"~/.ssh/ssh-agent-mux.sock"`

Path where ssh-agent-mux will create its multiplexed socket.

The `SSH_AUTH_SOCK` environment variable will be automatically set to this path.

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

The resolved absolute path to the multiplexed socket. This is automatically set as `SSH_AUTH_SOCK` in your environment.

## Service Management

The module creates a systemd user service that can be managed with standard systemctl commands.

### Managing the Service

After activating your home-manager configuration, the service will start automatically.

**Check service status:**
```bash
systemctl --user status ssh-agent-mux
```

**View logs:**
```bash
journalctl --user -u ssh-agent-mux -f
```

**Manually restart the service:**
```bash
systemctl --user restart ssh-agent-mux
```

**Manually stop the service:**
```bash
systemctl --user stop ssh-agent-mux
```

**Manually start the service:**
```bash
systemctl --user start ssh-agent-mux
```

## Environment Variables

The module automatically sets `SSH_AUTH_SOCK` via `home.sessionVariables` to point to the multiplexed socket.

You may need to reload your shell or re-login for the environment variable to take effect:

```bash
# Check if SSH_AUTH_SOCK is set correctly
echo $SSH_AUTH_SOCK
# Should output: /home/youruser/.ssh/ssh-agent-mux.sock (or your configured path)
```

## Security Features

The systemd service includes several security hardening options:

- `PrivateTmp=true` (unless `watchForSSHForward` is enabled) - Private /tmp directory
- `NoNewPrivileges=true` - Cannot gain new privileges
- `ProtectSystem=strict` - Read-only system directories
- `ProtectHome=read-only` - Read-only home directory (except socket directory)

## Example Configurations

### Basic Configuration with 1Password

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/.1password/agent.sock"
  ];
};
```

### Multiple Agents

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
    "~/.1password/agent.sock"
  ];
  watchForSSHForward = true;
  logLevel = "info";
};
```

### Custom Socket Location

```nix
services.ssh-agent-mux = {
  enable = true;
  agentSockets = [
    "~/.1password/agent.sock"
  ];
  listenPath = "~/.local/state/ssh-agent-mux.sock";
};
```

## Integration with NixOS

If you're using home-manager as a NixOS module:

```nix
# /etc/nixos/configuration.nix
{ config, pkgs, ... }:

{
  imports = [
    <home-manager/nixos>
  ];

  home-manager.users.myuser = { pkgs, ... }: {
    imports = [ inputs.ssh-agent-mux.homeManagerModules.default ];
    
    services.ssh-agent-mux = {
      enable = true;
      agentSockets = [
        "~/.1password/agent.sock"
      ];
    };
  };
}
```

## Troubleshooting

### Service won't start

Check the service status and logs:
```bash
systemctl --user status ssh-agent-mux
journalctl --user -u ssh-agent-mux -n 50
```

### SSH authentication not working

Verify the socket path and environment variable:
```bash
echo $SSH_AUTH_SOCK
ls -la $SSH_AUTH_SOCK
```

List available keys:
```bash
ssh-add -L
```

### Environment variable not set

Ensure you've reloaded your shell or re-logged in after activating home-manager:
```bash
# Re-source your shell config
source ~/.bashrc  # or ~/.zshrc
```

Or log out and log back in.

### No keys visible

Ensure the upstream agent sockets exist:
```bash
ls -la ~/.1password/agent.sock
```

Check if upstream agents have keys:
```bash
SSH_AUTH_SOCK=~/.1password/agent.sock ssh-add -L
```

### Permission denied errors

Check socket directory permissions:
```bash
ls -ld ~/.ssh
# Should be: drwx------ (700)
```

If needed, fix permissions:
```bash
chmod 700 ~/.ssh
```

## Comparison with nix-darwin Module

The home-manager module differs from the nix-darwin module in several ways:

- Uses **systemd** instead of launchd
- Logs to **journald** instead of files
- Includes **security hardening** options
- Automatically creates socket directory with correct permissions
- Integrates with home-manager's session variables

## See Also

- [ssh-agent-mux README](https://github.com/overhacked/ssh-agent-mux)
- [home-manager documentation](https://nix-community.github.io/home-manager/)
- [systemd user services](https://wiki.archlinux.org/title/Systemd/User)

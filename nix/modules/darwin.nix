# nix-darwin module for ssh-agent-mux
# Provides a launchd user service for macOS
{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.ssh-agent-mux;

  # Expand tilde to $HOME at runtime (launchd will expand it)
  expandPath = path:
    if hasPrefix "~/" path
    then "$HOME" + (removePrefix "~" path)
    else path;

  startScript = pkgs.writeShellScript "ssh-agent-mux-launchd-start" ''
    set -euo pipefail

    listen_path=${expandPath cfg.listenPath}
    listen_dir=$(dirname "$listen_path")
    mkdir -p "$listen_dir"
    rm -f "$listen_path"

    args=(
      --listen "$listen_path"
      --log-level ${cfg.logLevel}
    )

    ${optionalString cfg.watchForSSHForward "args+=(--watch-for-ssh-forward)"}

    ${
      concatMapStrings
      (socket: ''args+=("${expandPath socket}")\n'')
      cfg.agentSockets
    }

    exec ${cfg.package}/bin/ssh-agent-mux "''${args[@]}"
  '';
in {
  options.services.ssh-agent-mux = {
    enable = mkEnableOption "SSH Agent Mux service";

    agentSockets = mkOption {
      type = types.listOf types.str;
      default = [];
      description = lib.mdDoc ''
        List of agent socket paths to multiplex (order matters).

        The order of agent sockets affects the order in which public keys
        are offered to SSH servers during authentication.
      '';
      example = literalExpression ''
        [
          "~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"
          "~/Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh"
          "~/.ssh/yubikey-agent.sock"
        ]
      '';
    };

    listenPath = mkOption {
      type = types.str;
      default = "~/.ssh/ssh-agent-mux.sock";
      description = lib.mdDoc ''
        Path where ssh-agent-mux will create its multiplexed socket.

        Set your `SSH_AUTH_SOCK` environment variable to this path,
        or use `config.services.ssh-agent-mux.socketPath` in your shell configuration.
      '';
    };

    watchForSSHForward = mkOption {
      type = types.bool;
      default = false;
      description = lib.mdDoc ''
        Enable automatic detection of SSH forwarded agents.

        When enabled, ssh-agent-mux will watch for SSH agent sockets
        forwarded via `ssh -A` and automatically include them in the
        multiplexed agent. Forwarded agents are prioritized (newest first).
      '';
    };

    logLevel = mkOption {
      type = types.enum ["error" "warn" "info" "debug"];
      default = "info";
      description = lib.mdDoc ''
        Log level for ssh-agent-mux.

        Valid values: error, warn, info, debug
      '';
    };

    package = mkOption {
      type = types.package;
      default = pkgs.ssh-agent-mux or (throw "ssh-agent-mux package not found. Add the overlay or provide a custom package.");
      defaultText = literalExpression "pkgs.ssh-agent-mux";
      description = lib.mdDoc ''
        The ssh-agent-mux package to use.
      '';
    };

    socketPath = mkOption {
      type = types.str;
      readOnly = true;
      default = expandPath cfg.listenPath;
      description = lib.mdDoc ''
        Resolved absolute path to the multiplexed socket.

        Use this in your shell configuration:
        ```nix
        home.sessionVariables.SSH_AUTH_SOCK = config.services.ssh-agent-mux.socketPath;
        ```
      '';
    };
  };

  config = mkIf cfg.enable {
    # Ensure the package is available
    environment.systemPackages = [cfg.package];

    # Create launchd user agent
    launchd.user.agents.ssh-agent-mux = {
      serviceConfig = {
        ProgramArguments = [startScript];

        # Run at load and keep alive
        RunAtLoad = true;
        KeepAlive = true;

        # Background process type
        ProcessType = "Background";

        # Logging: Uses launchd defaults
        # - ~/Library/Logs/org.nixos.ssh-agent-mux.stdout.log
        # - ~/Library/Logs/org.nixos.ssh-agent-mux.stderr.log

        # Environment
        EnvironmentVariables = {
          PATH = "${cfg.package}/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        };

        # Nice value (lower priority)
        Nice = 5;

        # Throttling (restart delay)
        ThrottleInterval = 10;
      };
    };

    # Set up environment variable for user sessions
    # Note: This sets it system-wide for all user sessions
    environment.variables = {
      SSH_AUTH_SOCK = cfg.socketPath;
    };

    # Warnings
    warnings = optional (cfg.agentSockets == [] && !cfg.watchForSSHForward) ''
      services.ssh-agent-mux: No agent sockets configured and watchForSSHForward is disabled.
      The service will run but no SSH agents will be available.

      Configure agentSockets or enable watchForSSHForward.
    '';
  };
}

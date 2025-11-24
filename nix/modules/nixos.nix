# NixOS module for ssh-agent-mux
# Provides a systemd user service for Linux
{
  config,
  lib,
  pkgs,
  sshAgentMuxPackage ? null,
  ...
}:
with lib; let
  cfg = config.services.ssh-agent-mux;
  defaultPackage =
    if sshAgentMuxPackage != null then sshAgentMuxPackage
    else if pkgs ? ssh-agent-mux then pkgs.ssh-agent-mux
    else
      throw ''
        ssh-agent-mux package not found.

        When importing the module from the flake, add
        `overlays = [ ssh-agent-mux.overlays.default ];`
        or set `services.ssh-agent-mux.package` explicitly.
      '';

  # Convert "~/" paths to systemd specifiers so %h is used for the active user
  toSystemdPath = path:
    if hasPrefix "~/" path
    then "%h" + (removePrefix "~" path)
    else path;

  # Convert "~/" paths to $HOME for shell usage (e.g., ExecStart script, env vars)
  toShellPath = path:
    if hasPrefix "~/" path
    then "$HOME" + (removePrefix "~" path)
    else path;

  startScript = pkgs.writeShellScript "ssh-agent-mux-start" ''
    set -euo pipefail

    listen_path=${toShellPath cfg.listenPath}
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
      (socket: ''args+=("${toShellPath socket}")\n'')
      cfg.agentSockets
    }

    exec ${cfg.package}/bin/ssh-agent-mux "''${args[@]}"
  '';

  systemdSocketPath = toSystemdPath cfg.listenPath;
  socketDir = dirOf systemdSocketPath;
in {
  options.services.ssh-agent-mux = {
    enable = mkEnableOption "SSH Agent Mux user service";

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
          "~/.1password/agent.sock"
          "~/.ssh/yubikey-agent.sock"
        ]
      '';
    };

    listenPath = mkOption {
      type = types.str;
      default = "~/.ssh/ssh-agent-mux.sock";
      description = lib.mdDoc ''
        Path where ssh-agent-mux will create its multiplexed socket.

        Set `SSH_AUTH_SOCK` to this path (the module also exposes the resolved
        `socketPath` helper for use in shell configuration).
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
      default = defaultPackage;
      defaultText = literalExpression "pkgs.ssh-agent-mux";
      description = lib.mdDoc ''
        The ssh-agent-mux package to use.
      '';
    };

    socketPath = mkOption {
      type = types.str;
      readOnly = true;
      default = toShellPath cfg.listenPath;
      description = lib.mdDoc ''
        Resolved socket path that can be assigned to `SSH_AUTH_SOCK`.

        `~` is automatically expanded into ``$HOME`` for convenience.
      '';
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [cfg.package];

    systemd.user.services.ssh-agent-mux = {
      description = "SSH Agent Multiplexer";
      documentation = ["https://github.com/overhacked/ssh-agent-mux"];
      after = ["default.target"];
      wantedBy = ["default.target"];

      serviceConfig = {
        Type = "simple";
        ExecStart = startScript;
        Restart = "on-failure";
        RestartSec = "5s";

        # Private /tmp must be disabled when watching for forwarded agents so we can see them
        PrivateTmp = !cfg.watchForSSHForward;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths = [socketDir];
      };

      restartTriggers = [cfg.package startScript];
    };

    environment.sessionVariables.SSH_AUTH_SOCK = cfg.socketPath;

    warnings = optional (cfg.agentSockets == [] && !cfg.watchForSSHForward) ''
      services.ssh-agent-mux: No agent sockets configured and watchForSSHForward is disabled.
      The service will run but no SSH agents will be available.

      Configure agentSockets or enable watchForSSHForward.
    '';
  };
}

# home-manager module for ssh-agent-mux
# Provides a systemd user service for Linux/NixOS
{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.ssh-agent-mux;

  # Expand tilde to home directory
  expandPath = path:
    if hasPrefix "~/" path
    then "${config.home.homeDirectory}" + (removePrefix "~" path)
    else path;

  # Build command line arguments
  args =
    [
      "--listen"
      cfg.listenPath
      "--log-level"
      cfg.logLevel
    ]
    ++ (optionals cfg.watchForSSHForward ["--watch-for-ssh-forward"])
    ++ cfg.agentSockets;
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

        The `SSH_AUTH_SOCK` environment variable will be automatically
        set to this path via `home.sessionVariables`.
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

        This is automatically set as `SSH_AUTH_SOCK` in your environment.
      '';
    };
  };

  config = mkIf cfg.enable {
    # Ensure the package is available
    home.packages = [cfg.package];

    # Create systemd user service
    systemd.user.services.ssh-agent-mux = {
      Unit = {
        Description = "SSH Agent Multiplexer";
        Documentation = "https://github.com/overhacked/ssh-agent-mux";
        After = ["default.target"];
      };

      Service = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/ssh-agent-mux ${escapeShellArgs args}";
        Restart = "on-failure";
        RestartSec = "5s";

        # Security hardening
        PrivateTmp = true;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";

        # Allow writing to socket directory
        ReadWritePaths = [
          (dirOf cfg.socketPath)
        ];
      };

      Install = {
        WantedBy = ["default.target"];
      };
    };

    # Set SSH_AUTH_SOCK environment variable
    home.sessionVariables = {
      SSH_AUTH_SOCK = cfg.socketPath;
    };

    # Ensure socket directory exists with correct permissions
    home.file."${dirOf cfg.listenPath}/.keep" = mkIf (hasPrefix "~/" cfg.listenPath) {
      text = "";
      onChange = ''
        chmod 700 "${dirOf cfg.socketPath}"
      '';
    };

    # Warnings
    warnings = optional (cfg.agentSockets == [] && !cfg.watchForSSHForward) ''
      services.ssh-agent-mux: No agent sockets configured and watchForSSHForward is disabled.
      The service will run but no SSH agents will be available.

      Configure agentSockets or enable watchForSSHForward.
    '';
  };
}

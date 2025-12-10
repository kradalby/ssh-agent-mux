# home-manager module for ssh-agent-mux
# Provides a systemd user service for Linux/NixOS
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

  # Expand tilde to home directory
  expandPath = path:
    if hasPrefix "~/" path
    then "${config.home.homeDirectory}" + (removePrefix "~" path)
    else path;

  # Derive control socket path from listen path
  deriveControlPath = listenPath:
    let
      expanded = expandPath listenPath;
      base = if hasSuffix ".sock" expanded
        then removeSuffix ".sock" expanded
        else expanded;
    in "${base}.ctl";

  # Build command line arguments
  args =
    [
      "--listen"
      cfg.listenPath
      "--log-level"
      cfg.logLevel
    ]
    ++ (optionals (cfg.controlSocketPath != null) [
      "--control-socket"
      cfg.controlSocketPath
    ])
    ++ (optionals (cfg.healthCheckInterval > 0) [
      "--health-check-interval"
      (toString cfg.healthCheckInterval)
    ])
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

    controlSocketPath = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = lib.mdDoc ''
        Path for the control socket used by CLI commands.

        If not set, defaults to the listen path with `.ctl` extension
        instead of `.sock` (e.g., `~/.ssh/ssh-agent-mux.ctl`).
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

    healthCheckInterval = mkOption {
      type = types.ints.unsigned;
      default = 60;
      description = lib.mdDoc ''
        Interval in seconds between health checks of upstream agent sockets.

        Set to 0 to disable periodic health checks.
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
      default = expandPath cfg.listenPath;
      description = lib.mdDoc ''
        Resolved absolute path to the multiplexed socket.

        This is automatically set as `SSH_AUTH_SOCK` in your environment.
      '';
    };

    controlPath = mkOption {
      type = types.str;
      readOnly = true;
      default = if cfg.controlSocketPath != null
        then expandPath cfg.controlSocketPath
        else deriveControlPath cfg.listenPath;
      description = lib.mdDoc ''
        Resolved absolute path to the control socket.

        Used by CLI commands like `ssh-agent-mux status`.
      '';
    };

    watchdogSec = mkOption {
      type = types.ints.unsigned;
      default = 60;
      description = lib.mdDoc ''
        Watchdog interval in seconds for systemd health monitoring.

        The daemon pings at half this interval after completing health checks.
        If systemd doesn't receive a ping within this interval, it restarts the service.

        Set to 0 to disable systemd watchdog monitoring.
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
        # Use Type=notify for proper systemd integration
        # The daemon sends READY=1 when it's ready to accept connections
        Type = "notify";
        ExecStart = "${cfg.package}/bin/ssh-agent-mux ${escapeShellArgs args}";
        Restart = "on-failure";
        RestartSec = "5s";

        # Watchdog: systemd will restart the service if it doesn't receive
        # periodic pings within this interval (the daemon pings during health checks)
        WatchdogSec = mkIf (cfg.watchdogSec > 0) cfg.watchdogSec;

        # Disable private /tmp when we need to inspect forwarded agents
        PrivateTmp = !cfg.watchForSSHForward;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";

        # Allow writing to socket directory (for both listen and control sockets)
        ReadWritePaths = [
          (dirOf cfg.socketPath)
          (dirOf cfg.controlPath)
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

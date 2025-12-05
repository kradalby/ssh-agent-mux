{
  description = "SSH Agent Mux - Combine keys from multiple SSH agents into a single agent socket";

  inputs = {
    # For listing and iterating nix systems
    flake-utils.url = "github:numtide/flake-utils";

    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    # For installing non-standard rustc versions
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
  }:
    {
      overlays.default = final: prev: {
        ssh-agent-mux = self.packages.${final.system}.ssh-agent-mux;
      };

      # NixOS module for systemd user service
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      } @ args:
        let
          module = import ./nix/modules/nixos.nix;
        in
          module
          (args
            // {
              sshAgentMuxPackage =
                self.packages.${pkgs.stdenv.hostPlatform.system}.ssh-agent-mux;
            });

      # Darwin module for macOS (nix-darwin)
      darwinModules.default = {
        config,
        lib,
        pkgs,
        ...
      } @ args:
        let
          module = import ./nix/modules/darwin.nix;
        in
          module
          (args
            // {
              sshAgentMuxPackage =
                self.packages.${pkgs.stdenv.hostPlatform.system}.ssh-agent-mux;
            });

      # Home Manager module for Linux/NixOS
      homeManagerModules.default = {
        config,
        lib,
        pkgs,
        ...
      } @ args:
        let
          module = import ./nix/modules/home-manager.nix;
        in
          module
          (args
            // {
              sshAgentMuxPackage =
                self.packages.${pkgs.stdenv.hostPlatform.system}.ssh-agent-mux;
            });
    }
    // (flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [
          rust-overlay.overlays.default
        ];
      };

      # Read version from Cargo.toml
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      packageVersion = cargoToml.package.version;
      rustVersion = cargoToml.package.rust-version;

      filterSrc = src: regexes:
        pkgs.lib.cleanSourceWith {
          inherit src;
          filter = path: type: let
            relPath = pkgs.lib.removePrefix (toString src + "/") (toString path);
          in
            pkgs.lib.all (re: builtins.match re relPath == null) regexes;
        };

      # Shell toolchain: specified version with rust-src and rust-analyzer for development
      rustShellToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
        extensions = ["rust-src" "rust-analyzer"];
      };

      # Build toolchain: minimal specified version for CI and package builds
      rustMinimalToolchain = pkgs.rust-bin.stable.${rustVersion}.minimal;

      rustMinimalPlatform = pkgs.makeRustPlatform {
        rustc = rustMinimalToolchain;
        cargo = rustMinimalToolchain;
      };

      nativeBuildInputs = with pkgs;
        []
        ++ lib.optionals stdenv.isLinux [
          mold-wrapped
        ];

      buildInputs = [];

      nativeCheckInputs = with pkgs; [
        # for SSH agent tests
        openssh

        # for signing tests (if needed in future)
        gnupg
      ];

      env = {
        RUST_BACKTRACE = 1;
        CARGO_INCREMENTAL = "0"; # https://github.com/rust-lang/rust/issues/139110
      };
    in {
      formatter = pkgs.alejandra;

      packages = {
        ssh-agent-mux = rustMinimalPlatform.buildRustPackage {
          pname = "ssh-agent-mux";
          version = "${packageVersion}";

          src = filterSrc ./. [
            ".*\\.nix$"
            "^.github/"
            "^flake\\.lock$"
            "^target/"
            "^result$"
            "^nix/"
          ];

          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs nativeCheckInputs;

          # Enable systemd feature on Linux for sd_notify support
          buildFeatures = pkgs.lib.optionals pkgs.stdenv.isLinux ["systemd"];

          # Disable tests during build - they timeout in Nix sandbox
          doCheck = false;

          env =
            env
            // {
              RUSTFLAGS = pkgs.lib.optionalString pkgs.stdenv.isLinux "-C link-arg=-fuse-ld=mold";
            };

          meta = {
            description = "Combine keys from multiple SSH agents into a single agent socket";
            homepage = "https://github.com/overhacked/ssh-agent-mux";
            license = with pkgs.lib.licenses; [asl20 bsd3];
            mainProgram = "ssh-agent-mux";
            maintainers = [];
          };
        };
        default = self.packages.${system}.ssh-agent-mux;
      };

      checks.ssh-agent-mux = self.packages.${system}.ssh-agent-mux.overrideAttrs ({...}: {
        # Run checks under the test profile for faster builds
        cargoBuildType = "test";
        cargoCheckType = "test";

        # We don't care about the binary in checks, just that tests pass
        buildPhase = "true";
        installPhase = "touch $out";
      });

      devShells.default = let
        packages = with pkgs;
          [
            rustShellToolchain

            # Testing tools
            cargo-nextest

            # Development tools
            cargo-watch
            bacon

            # Security audit
            cargo-audit

            # Code coverage (optional)
            cargo-tarpaulin
          ]
          ++ nativeBuildInputs
          ++ buildInputs
          ++ nativeCheckInputs;

        # Platform-specific linker flags for faster builds
        rustLinkerFlags =
          if pkgs.stdenv.isLinux
          then ["-fuse-ld=mold" "-Wl,--compress-debug-sections=zstd"]
          else if pkgs.stdenv.isDarwin
          then
            # On macOS, use the modern ld_new linker for faster linking
            ["--ld-path=$(unset DEVELOPER_DIR; /usr/bin/xcrun --find ld)" "-ld_new"]
          else [];

        rustLinkFlagsString =
          pkgs.lib.concatStringsSep " "
          (pkgs.lib.concatMap (x: ["-C" "link-arg=${x}"]) rustLinkerFlags);

        # Set RUSTFLAGS in shellHook to allow shell interpretation (xcrun on macOS)
        shellHook = ''
          export RUSTFLAGS="${rustLinkFlagsString}"
        '';
      in
        pkgs.mkShell {
          name = "ssh-agent-mux";
          inherit packages env shellHook;
        };
    }));
}

{
  description = "Hyperion - A Minecraft bot framework";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Minecraft 26.x refuses to run on anything below JDK 25, which the pinned
    # nixpkgs above predates (it tops out at jdk23). A second input supplies the
    # toolchain for the protocol pipeline without forcing a repo-wide bump that
    # would rebuild every Rust dependency.
    nixpkgs-minecraft.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, nixpkgs-minecraft, rust-overlay, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      mkSystem = system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };

          rustToolchain = pkgs.rust-bin.nightly."2025-02-22".default.override {
            extensions = [ "rust-src" "rustfmt" "clippy" ];
          };

          # Minecraft protocol data: fetching the pinned server jar, running
          # Mojang's data generator, and generating the Rust tables from it.
          #
          # Anything derived from the server jar is unfree under Mojang's EULA,
          # so it gets its own nixpkgs instance whose unfree allowance is
          # narrowed to exactly those derivations. Flipping allowUnfree on the
          # shared instance would quietly relax the policy for the whole flake.
          minecraftPkgs = import nixpkgs-minecraft {
            inherit system;
            config.allowUnfreePredicate =
              pkg: nixpkgs.lib.hasPrefix "minecraft-" (nixpkgs.lib.getName pkg);
          };

          minecraft = import ./nix/minecraft-data.nix { pkgs = minecraftPkgs; };

          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
            cmake
          ];

          buildInputs = with pkgs; [
            openssl
          ] ++ lib.optionals stdenv.isDarwin [
            darwin.apple_sdk.frameworks.Security
            darwin.apple_sdk.frameworks.SystemConfiguration
          ];

          hyperion = pkgs.rustPlatform.buildRustPackage {
            pname = "hyperion";
            version = "0.1.0";
            src = ./.;

            inherit buildInputs nativeBuildInputs;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "bvh-0.1.0" = "sha256-KHQ7Uh1Y4mGIYj16aX36dy927pf401bQFNKBnL+VwCo=";
                "divan-0.1.17" = "sha256-0zrZsUAqU7f53FEPtAdueOD3rl+G0ekYRKoVEehneNg=";
                "flecs_ecs-0.1.3" = "sha256-A4gLBl9aK/ThXdkIslouooKn/7jKbfl8OSfg0BRyLT4=";
                "valence_anvil-0.1.0" = "sha256-sirOc/aNOCbkzvf/igm7PTA1+YOMgj9ov2BINprxNa0=";
              };
            };
          };

          # Create minimal runtime environment
          minimalEnv = pkgs.buildEnv {
            name = "minimal-env";
            paths = [
              (pkgs.runCommand "hyperion-bins" { } ''
                mkdir -p $out/bin
                cp ${hyperion}/bin/hyperion-proxy $out/bin/
                cp ${hyperion}/bin/bedwars $out/bin/
              '')
              pkgs.cacert # Required for SSL/TLS
            ];
          };

          # Docker image for hyperion-proxy
          hyperion-proxy-image = pkgs.dockerTools.buildLayeredImage {
            name = "hyperion-proxy";
            tag = "latest";
            maxLayers = 5;
            contents = [ minimalEnv ];

            config = {
              Cmd = [ "/bin/hyperion-proxy" "0.0.0.0:8080" ];
              ExposedPorts = {
                "8080/tcp" = { };
              };
            };
          };

          # Docker image for bedwars
          bedwars-image = pkgs.dockerTools.buildLayeredImage {
            name = "bedwars";
            tag = "latest";
            maxLayers = 5;
            contents = [ minimalEnv ];

            config = {
              Cmd = [ "/bin/bedwars" "--ip" "0.0.0.0" "--port" "35565" ];
              ExposedPorts = {
                "35565/tcp" = { };
              };
            };
          };
        in
        {
          devShells.default = pkgs.mkShell {
            inherit buildInputs nativeBuildInputs;
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          };

          packages = {
            default = hyperion;
            docker-hyperion-proxy = hyperion-proxy-image;
            docker-bedwars = bedwars-image;

            minecraft-server-jar = minecraft.serverJar;
            minecraft-data = minecraft.generatedData;
            minecraft-decompiled = minecraft.decompiledSources;
            minecraft-protocol = minecraft.protocolJson;
            minecraft-proto-rust = minecraft.generatedRust;
          };

          apps = {
            # Re-resolves Mojang's manifest and rewrites nix/minecraft-version.json.
            update-minecraft-data = {
              type = "app";
              program = pkgs.lib.getExe minecraft.updateScript;
            };
            # Copies the generated Rust into the crate so cargo works without nix.
            sync-minecraft-proto = {
              type = "app";
              program = pkgs.lib.getExe minecraft.syncScript;
            };
            extract-minecraft-protocol = {
              type = "app";
              program = pkgs.lib.getExe minecraft.extractor;
            };
          };

          checks = {
            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-protocol = minecraft.protocolJson;
          };
        };
    in
    {
      devShells = forAllSystems (system: (mkSystem system).devShells);
      packages = forAllSystems (system: (mkSystem system).packages);
      apps = forAllSystems (system: (mkSystem system).apps);
      checks = forAllSystems (system: (mkSystem system).checks);
    };
}

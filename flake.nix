{
  description = "Hyperion - A Minecraft bot framework";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # The bare-metal target needs nightly-2026-07-01, which the pinned
    # rust-overlay above predates. Kept as a second input so that bumping it
    # cannot move the toolchain every normal build uses.
    rust-overlay-bare-metal = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, rust-overlay-bare-metal, ... }:
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
          # Experimental: hyperion's protocol layer as a unikernel image, with
          # no operating system under it. Pinned to the Hermit kernel's own
          # nightly, which is not the one the rest of the tree uses; see
          # docs/bare-metal.md.
          bareMetal = pkgs.callPackage ./nix/bare-metal.nix {
            rustToolchain =
              (import nixpkgs {
                inherit system;
                overlays = [ (import rust-overlay-bare-metal) ];
              }).rust-bin.nightly."2026-07-01".default.override {
                extensions = [ "rust-src" "llvm-tools" ];
                # The kernel's loader stub is built for x86_64-unknown-none.
                targets = [ "x86_64-unknown-none" ];
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
            bare-metal = bareMetal.image;
            bare-metal-vendor = bareMetal.image.passthru.vendor;
          };

          apps.bare-metal-vm = {
            type = "app";
            program = pkgs.lib.getExe bareMetal.vm;
          };
        };
    in
    {
      apps = forAllSystems (system: (mkSystem system).apps);
      devShells = forAllSystems (system: (mkSystem system).devShells);
      packages = forAllSystems (system: (mkSystem system).packages);
    };
}

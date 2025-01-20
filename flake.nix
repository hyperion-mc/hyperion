{
  description = "Hyperion - A Minecraft bot framework";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.nightly."2024-12-18".default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
        };

        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          cmake
          llvmPackages.libclang
        ];

        # GenMap derivation
        genmap = pkgs.stdenv.mkDerivation {
          name = "genmap";
          src = pkgs.fetchurl {
            url = "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";
            sha256 = "sha256-ViX+qDEBf++HwbXu1rTAi05Ju/3JAS16Ld4Uq0sStQg=";
          };

          installPhase = ''
            mkdir -p $out
            cp -r . $out/
          '';
        };

        buildInputs = with pkgs; [
          openssl
          genmap
        ] ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          HYPERION_GENMAP_PATH = "${genmap}";
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "hyperion";
          version = "0.1.0";
          src = ./.;

          inherit buildInputs nativeBuildInputs;
          HYPERION_GENMAP_PATH = "${genmap}";

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "bvh-0.1.0" = "sha256-yOsM6r96zOE0LD0JRWushzrxDVqncXHzZvrnOm7xNGc=";
              "divan-0.1.17" = "sha256-UZNINS/JOgQfUUlJf8AUZkUuLH2y6tCZsDt0TasrYb0=";
              "flecs_ecs-0.1.3" = "sha256-YzvlWJzh/K5rkn590BD5Q/GWYx9JDU+KPXqnB0tUc9U=";
              "valence_anvil-0.1.0" = "sha256-0ALeK1kCgusExf57ssPDkKinu8iNeveCBoV9hMBB/Y8=";
            };
          };

          #   checkPhase = ''
          #     runHook preCheck
          #     cargo test
          #     cargo clippy -- -D warnings
          #     cargo fmt --check
          #     cargo deny check
          #     runHook postCheck
          #   '';
        };
      }
    );
}

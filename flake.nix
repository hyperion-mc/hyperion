{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        # Fetch git dependencies
        flecsRust = pkgs.fetchFromGitHub {
          owner = "Indra-db";
          repo = "Flecs-Rust";
          rev = "1480395bf6149d185473cd7ef69b88a58fa315b4";
          sha256 = "sha256-IYx2/q6H5klkhkDtKaEh/WPTFAQQN7xXD5LIwa9WvdU=";
        };

        bvhData = pkgs.fetchFromGitHub {
          owner = "andrewgazelka";
          repo = "bvh-data";
          rev = "915b6ec0fd655b6a01f35e2d8fc658ece03496d0";
          sha256 = "sha256-yOsM6r96zOE0LD0JRWushzrxDVqncXHzZvrnOm7xNGc=";
        };

        divanBench = pkgs.fetchFromGitHub {
          owner = "nvzqz";
          repo = "divan";
          rev = "98d6e68c5f90ce47cfac1059bdfbf1fb228a76dd";
          sha256 = "sha256-UZNINS/JOgQfUUlJf8AUZkUuLH2y6tCZsDt0TasrYb0=";
        };

        valenceRepo = pkgs.fetchFromGitHub {
          owner = "andrewgazelka";
          repo = "valence";
          rev = "7ed3252c1172c935f8e56df0699339ab35b08f65";
          sha256 = "sha256-0ALeK1kCgusExf57ssPDkKinu8iNeveCBoV9hMBB/Y8=";
        };

        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          libiconv # Required for macOS
        ];

        buildPackage = profile:
          pkgs.stdenv.mkDerivation {
            pname = "hyperion";
            version = "0.1.0";
            src = ./.;

            inherit nativeBuildInputs;

            # Configure cargo to use our fetched dependencies
            preBuild = ''
              # Set cargo home to a writable location within the build directory
              export CARGO_HOME="$PWD/.cargo-home"
              mkdir -p $CARGO_HOME

              # Create vendor directory
              mkdir -p vendor

              # Create initial .cargo/config.toml for git dependencies and vendored sources
              mkdir -p .cargo
              cat > .cargo/config.toml << EOF
              [source.crates-io]
              replace-with = "vendored-sources"

              [source."vendored-sources"]
              directory = "vendor"

              [source."https://github.com/Indra-db/Flecs-Rust"]
              git = "file://${flecsRust}"
              branch = "master"

              [source."https://github.com/andrewgazelka/bvh-data"]
              git = "file://${bvhData}"
              branch = "master"

              [source."https://github.com/nvzqz/divan"]
              git = "file://${divanBench}"
              branch = "master"

              [source."https://github.com/andrewgazelka/valence"]
              git = "file://${valenceRepo}"
              branch = "feat-open"
              EOF

              # Download dependencies and vendor them
              cargo vendor > vendor-config

              # Append the vendor config but keep the crates-io replacement
              grep -v '\[source.crates-io\]' vendor-config | grep -v 'replace-with' >> .cargo/config.toml
            '';

            buildPhase = ''
              cargo build --frozen --profile ${profile} --workspace
            '';

            installPhase = ''
              mkdir -p $out/bin
              find target/${profile} -type f -executable -exec cp {} $out/bin/ \;
            '';

            # Required for macOS builds
            propagatedBuildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
              pkgs.darwin.apple_sdk.frameworks.CoreFoundation
            ];
          };

      in {
        packages = {
          default = buildPackage "release-full";
          release = buildPackage "release";
          release-debug = buildPackage "release-debug";
          release-full = buildPackage "release-full";
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;

          buildInputs = with pkgs; [ cargo-nextest cargo-watch ];

          shellHook = ''
            export RUST_BACKTRACE=1
            export PATH=$PATH:''${CARGO_HOME:-~/.cargo}/bin
          '';

          # Required for macOS builds
          propagatedBuildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            pkgs.darwin.apple_sdk.frameworks.CoreFoundation
          ];
        };
      });
}

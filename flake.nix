{
  description = "Hyperion - A Minecraft game engine";

  nixConfig = {
    extra-substituters = [ "https://cache.ix.dev" ];
    extra-trusted-public-keys = [
      "ix-workspace:JuAaeOPfR3GL3nUICpEz/88/+S3BzGF3L6bPYFy0GwI="
    ];
    # cargoUnit content-addresses every crate unit, so a source change that does
    # not change a crate's output stops the rebuild there instead of at the root.
    extra-experimental-features = [ "ca-derivations" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    index = {
      url = "github:indexable-inc/index";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, index, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      # rust-toolchain.toml is the one place the version lives; rustup users and
      # CI already read it, so the flake reads it too rather than restating it.
      rustChannel = (nixpkgs.lib.importTOML ./rust-toolchain.toml).toolchain.channel;

      # Raising this is what keeps the proxy from running out of sockets once a
      # few thousand bots connect.
      fileDescriptors = "32768";

      clippyArgs = "--all-targets --all-features";

      mkSystem = system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          inherit (pkgs) lib;

          rustWith = components: pkgs.rust-bin.fromRustupToolchain {
            channel = rustChannel;
            inherit components;
          };

          # Anything derived from Mojang's server jar is unfree under their EULA,
          # so it gets an instance whose unfree allowance is narrowed to exactly
          # those derivations. Flipping allowUnfree on the shared instance would
          # quietly relax the policy for the whole flake.
          minecraftPkgs = import nixpkgs {
            inherit system;
            config.allowUnfreePredicate =
              pkg: nixpkgs.lib.hasPrefix "minecraft-" (nixpkgs.lib.getName pkg);
          };

          minecraft = import ./nix/minecraft-data.nix { pkgs = minecraftPkgs; };

          rustToolchain = rustWith [ "rustfmt" "clippy" "rust-src" ];
          rustWithMiri = rustWith [ "rustfmt" "clippy" "rust-src" "miri" ];
          rustWithCoverage = rustWith [ "rustfmt" "clippy" "rust-src" "llvm-tools-preview" ];

          cargoTools = [
            pkgs.cargo-deny
            pkgs.cargo-machete
            pkgs.cargo-nextest
            pkgs.cargo-watch
          ];

          nativeBuildInputs = [
            pkgs.cmake
            pkgs.pkg-config
          ];

          # Every dev command carries the tools it needs, so `nix run .#lint`
          # works on a machine with nothing but nix installed.
          mkScript = name: { text, deps ? [ ], toolchain ? rustToolchain }:
            pkgs.writeShellApplication {
              inherit name text;
              runtimeInputs = [ toolchain ] ++ deps;
            };

          checkScripts = lib.mapAttrs mkScript {
            # `flecs_ecs_sys`'s default features include `regenerate_binding`,
            # whose build script writes bindgen output into its own source
            # directory -- inside the shared cargo registry every checkout on
            # the machine reads. Cargo unifies features across the graph, so one
            # dependency pulling the sys crate with default features corrupts
            # that copy for everyone. `flecs_ecs` sets default-features = false;
            # this fails the moment anything reintroduces them. See ENG-10307.
            hot-reload-registry-guard.text = ''
              tree=$(cargo tree -p hyperion-hot-reload -e features -i flecs_ecs_sys)
              if grep -q 'regenerate_binding' <<<"$tree"; then
                echo "FAIL: flecs_ecs_sys resolved with regenerate_binding." >&2
                echo "Its build script writes into the shared cargo registry." >&2
                grep -n 'regenerate_binding' <<<"$tree" >&2
                exit 1
              fi
              echo "ok: flecs_ecs_sys has no regenerate_binding in the resolved graph"
            '';

            fmt.text = ''cargo fmt --all "$@"'';

            lint.text = ''cargo clippy ${clippyArgs} -- -D warnings'';

            lint-fix.text = ''
              cargo clippy --fix --allow-dirty --allow-staged ${clippyArgs} -- -D warnings
            '';

            test = {
              deps = [ pkgs.cargo-nextest ];
              text = ''cargo nextest run "$@"'';
            };

            # Only tests whose name contains "miri" run under it; the rest are
            # far too slow to interpret.
            miri = {
              deps = [ pkgs.cargo-nextest ];
              toolchain = rustWithMiri;
              text = ''
                export MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-ignore-leaks'
                cargo miri nextest run miri "$@"
              '';
            };

            deny = {
              deps = [ pkgs.cargo-deny ];
              text = ''cargo deny check "$@"'';
            };

            unused-deps = {
              deps = [ pkgs.cargo-machete ];
              text = ''cargo machete'';
            };

            doc.text = ''cargo doc --workspace --no-deps --all-features "$@"'';

            coverage = {
              deps = [ pkgs.cargo-llvm-cov ];
              toolchain = rustWithCoverage;
              text = ''
                cargo llvm-cov --all-features --workspace --branch \
                  --lcov --output-path lcov.info "$@"
              '';
            };
          };

          # The three cheap checks run together and every one of them reports
          # before the run fails, so a single push can fix all of them.
          ci = mkScript "ci" {
            text = ''
              pids=()
              "${lib.getExe checkScripts.fmt}" --check & pids+=("$!")
              "${lib.getExe checkScripts.unused-deps}" & pids+=("$!")
              "${lib.getExe checkScripts.deny}" & pids+=("$!")

              status=0
              for pid in "''${pids[@]}"; do
                wait "$pid" || status=1
              done
              [ "$status" -eq 0 ]

              "${lib.getExe checkScripts.lint}"
              "${lib.getExe checkScripts.test}"
              "${lib.getExe checkScripts.doc}"
            '';
          };

          runners = lib.mapAttrs mkScript {
            # Builds four successive versions of the demo game module and drives
            # one running world through all of them. Being a nix app is what
            # makes the single-compiler precondition structural rather than a
            # convention: `repr(Rust)` has no stable ABI, so a host and a module
            # built by different rustc versions disagree about the layout of
            # every type they share.
            hot-reload-demo = {
              deps = [ pkgs.cmake pkgs.pkg-config ];
              text = ''exec ./crates/hyperion-hot-reload/demo.sh "$@"'';
            };

            proxy.text = ''
              ulimit -Sn ${fileDescriptors}
              exec cargo run --profile release-full --bin hyperion-proxy -- \
                --server 127.0.0.1:35565 0.0.0.0:25565
            '';

            bedwars.text = ''
              exec cargo run --profile release-full -p bedwars -- "$@"
            '';

            # rust-mc-bot reads its whole configuration from BOT_-prefixed
            # environment variables, so the address and count cannot be passed
            # positionally to the binary.
            bots.text = ''
              export BOT_SERVER="''${1:-127.0.0.1:25565}"
              export BOT_BOT_COUNT="''${2:-100}"
              ulimit -Sn ${fileDescriptors}
              exec cargo run --release -p rust-mc-bot
            '';

            # One rebuild-and-restart loop for any profile: `nix run .#dev`, or
            # `nix run .#dev -- release-full`. One watcher rebuilds bedwars and
            # touches a trigger file; a second watches only the trigger, which is
            # what stops a restart from racing a half-written executable.
            dev = {
              deps = [ pkgs.cargo-watch pkgs.git pkgs.parallel ];
              text = ''
                profile="''${1:-dev}"
                case "$profile" in
                  dev | debug) profile=dev; target=debug ;;
                  *) target="$profile" ;;
                esac

                root="$(git rev-parse --show-toplevel)"
                trigger="$root/.trigger-$target"
                touch "$trigger"

                ulimit -Sn ${fileDescriptors}
                export RUST_BACKTRACE=full

                parallel --ungroup --halt now,done=1 --jobs 3 ::: \
                  "cargo watch --postpone --no-vcs-ignores -w '$trigger' -s '$root/target/$target/bedwars'" \
                  "cargo run --profile $profile --bin hyperion-proxy -- --server 127.0.0.1:35565 0.0.0.0:25565" \
                  "cargo watch -w '$root/crates/hyperion' -w '$root/events/bedwars' -s 'cargo build --profile $profile -p bedwars' -s 'touch $trigger'"
              '';
            };
          };

          scripts = checkScripts // runners // { inherit ci; };

          cargoUnit = index.lib.cargoUnitExternal {
            inherit pkgs rustToolchain;
          };

          workspace = cargoUnit.buildWorkspace {
            pname = "hyperion";
            src = ./.;
            workspaceRoot = ./.;
            cargoLock = ./Cargo.lock;
            cargoArgs = [ "--workspace" ];

            inherit nativeBuildInputs;

            # Linting, auditing and unused-dependency checks are the `nix run`
            # apps above, which use the pinned nightly. cargoUnit's own gates
            # would run a second, mismatched clippy over the same code.
            policy = {
              clippy.enable = false;
              cargoAudit.enable = false;
              cargoMachete.enable = false;
              denyUnusedCrateDependencies = false;
            };

            # Keyed by the exact source string in Cargo.lock. Refresh with
            # `nix-prefetch-git --fetch-submodules --url <url> --rev <rev>`.
            outputHashes = {
              "git+https://github.com/TestingPlant/bvh-data#9bffb03a4b894a7884c9ec0da986bdde732ac704" =
                "sha256-QjsyP9XdR53JDNFC8IX1qgTlJQZmanAZU+246QG4v9s=";
              "git+https://github.com/nvzqz/divan#bca5c9676a35751d0a8164df7d79bda70f23286b" =
                "sha256-WmzYLzLwXUGuX0K151Kh+fEV6nJJQLq/vb4ijXu01Vg=";
              "git+https://github.com/TestingPlant/valence?branch=feat-bytes#fb792dcb6669b64c5dc2366eb3d074b293def046" =
                "sha256-rpuJSz8KxEwG5qeT4HYVtTxHJ24nrYZJwDurv+mjPxM=";
              "git+https://github.com/TestingPlant/valence?branch=feat-open#7c664716cd1e7b30de4e38cfc0ee8d1ecc7b0bd5" =
                "sha256-BV6QgM5d5qanEGonbAV7iOhNDk4aW3ub3++DH7/DY5E=";
            };
          };
        in
        {
          devShells.default = pkgs.mkShell {
            nativeBuildInputs = nativeBuildInputs ++ cargoTools ++ [ rustToolchain ];
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          };

          apps = lib.mapAttrs
            (_: script: {
              type = "app";
              program = lib.getExe script;
            })
            (scripts // {
              default = scripts.dev;
              update-minecraft-data = minecraft.updateScript;
              sync-minecraft-proto = minecraft.syncScript;
              extract-minecraft-protocol = minecraft.extractor;
            });

          packages = {
            default = workspace.binaries.bedwars;
            inherit (workspace.binaries) bedwars hyperion-proxy rust-mc-bot;

            minecraft-server-jar = minecraft.serverJar;
            minecraft-data = minecraft.generatedData;
            minecraft-decompiled = minecraft.decompiledSources;
            minecraft-protocol = minecraft.protocolJson;
            minecraft-proto-rust = minecraft.generatedRust;
          };

          # `nix flake check` builds every app, which is what proves each one
          # passes shellcheck and that its tools resolve.
          checks = scripts // {
            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-protocol = minecraft.protocolJson;
          };

        };
    in
    {
      apps = forAllSystems (system: (mkSystem system).apps);
      checks = forAllSystems (system: (mkSystem system).checks);
      devShells = forAllSystems (system: (mkSystem system).devShells);
      packages = forAllSystems (system: (mkSystem system).packages);
    };
}

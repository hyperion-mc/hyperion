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

          rustToolchain = rustWith [ "rustfmt" "clippy" "rust-src" ];
          rustWithMiri = rustWith [ "rustfmt" "clippy" "rust-src" "miri" ];
          rustWithCoverage = rustWith [ "rustfmt" "clippy" "rust-src" "llvm-tools-preview" ];

          minecraft = import ./nix/minecraft-data.nix {
            pkgs = minecraftPkgs;
            rustfmt = rustToolchain;
          };

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

          certsDir = ".hyperion-dev-certs";

          # Defaults, not constants: HYPERION_PLAYER_PORT and
          # HYPERION_SERVER_PORT override them, and process-compose's own API
          # port moves with them. Two checkouts on one machine otherwise fight
          # over all three, and the loser dies on "address already in use".
          gameServerPort = 35565;
          proxyPort = 25565;

          # Generated rather than committed as YAML: the ports and cert paths
          # then have one source of truth shared with the standalone apps above.
          # Commands are single-line: a backslash continuation is literal in a
          # Nix indented string, survives into the YAML, and reaches the shell
          # as a stray argument rather than a line join.
          processComposeConfig = (pkgs.formats.yaml { }).generate "process-compose.yaml" {
            version = "0.5";
            processes = {
              game-server = {
                command = "cargo run --profile \"$\{HYPERION_PROFILE:-dev}\" -p \"$\{HYPERION_EVENT:-bedwars}\" -- --ip 0.0.0.0 --port \"$\{HYPERION_SERVER_PORT:-${toString gameServerPort}}\" --root-ca-cert ${certsDir}/root_ca.crt --cert ${certsDir}/server.crt --private-key ${certsDir}/server_private_key.pem";
                availability.restart = "on_failure";
              };

              proxy = {
                command = "ulimit -Sn ${fileDescriptors}; exec cargo run --profile \"$\{HYPERION_PROFILE:-dev}\" --bin hyperion-proxy -- --server 127.0.0.1:\"$\{HYPERION_SERVER_PORT:-${toString gameServerPort}}\" --root-ca-cert ${certsDir}/root_ca.crt --cert ${certsDir}/proxy.crt --private-key ${certsDir}/proxy_private_key.pem 0.0.0.0:\"$\{HYPERION_PLAYER_PORT:-${toString proxyPort}}\"";
                # Started, not healthy: a TCP readiness probe would connect and
                # immediately disconnect every few seconds, and the game server
                # logs an error for each half-open connection. The proxy already
                # retries until the game server answers.
                depends_on.game-server.condition = "process_started";
                availability.restart = "on_failure";
              };
            };
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

            # The game server and the proxy authenticate to each other with
            # mTLS, so a fresh clone cannot run until a CA and two leaf certs
            # exist. That is the only thing between `git clone` and a running
            # server, so it is a command rather than a page of README.
            certs = {
              deps = [ pkgs.openssl pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                dir="$root/${certsDir}"
                if [ -f "$dir/root_ca.crt" ] && [ "''${1:-}" != "--force" ]; then
                  echo "dev certificates already present in ${certsDir}"
                  echo "regenerate with: nix run .#certs -- --force"
                  exit 0
                fi
                mkdir -p "$dir"
                cd "$dir"

                openssl req -new -nodes -newkey rsa:2048 -keyout root_ca.pem \
                  -x509 -out root_ca.crt -days 365 -subj '/CN=hyperion-dev-ca'

                for who in server proxy; do
                  openssl req -nodes -newkey rsa:2048 \
                    -keyout "''${who}_private_key.pem" -out "$who.csr" \
                    -subj "/CN=hyperion-dev-$who"
                  # The SAN must cover the address the peer dials or the
                  # handshake fails with "certificate not valid for name".
                  openssl x509 -req -in "$who.csr" -CA root_ca.crt -CAkey root_ca.pem \
                    -CAcreateserial -out "$who.crt" -days 365 -sha256 \
                    -extfile <(printf 'subjectAltName=DNS:localhost,IP:127.0.0.1')
                  rm -f "$who.csr"
                done
                rm -f root_ca.srl

                echo "wrote throwaway dev certificates to ${certsDir}"
                echo "local development only -- never deploy these"
              '';
            };

            proxy = {
              deps = [ pkgs.git ];
              text = ''
                certs="$(git rev-parse --show-toplevel)/${certsDir}"
                ulimit -Sn ${fileDescriptors}
                exec cargo run --profile release-full --bin hyperion-proxy -- \
                  --server 127.0.0.1:35565 \
                  --root-ca-cert "$certs/root_ca.crt" \
                  --cert "$certs/proxy.crt" \
                  --private-key "$certs/proxy_private_key.pem" \
                  0.0.0.0:25565
              '';
            };

            bedwars = {
              deps = [ pkgs.git ];
              text = ''
                certs="$(git rev-parse --show-toplevel)/${certsDir}"
                exec cargo run --profile release-full -p bedwars -- \
                  --ip 0.0.0.0 --port 35565 \
                  --root-ca-cert "$certs/root_ca.crt" \
                  --cert "$certs/server.crt" \
                  --private-key "$certs/server_private_key.pem" \
                  "$@"
              '';
            };

            # Super Smash Mobs, the second event. Same shape as bedwars: one
            # game server per process, each binding its own port, so the two
            # are selected at run time rather than sharing anything.
            smash = {
              deps = [ pkgs.git ];
              text = ''
                certs="$(git rev-parse --show-toplevel)/${certsDir}"
                exec cargo run --profile release-full -p smash -- \
                  --ip 0.0.0.0 --port "''${HYPERION_SERVER_PORT:-${toString gameServerPort}}" \
                  --root-ca-cert "$certs/root_ca.crt" \
                  --cert "$certs/server.crt" \
                  --private-key "$certs/server_private_key.pem" \
                  "$@"
              '';
            };

            bots.text = ''
              ulimit -Sn ${fileDescriptors}
              exec cargo run --release -p rust-mc-bot -- \
                "''${1:-127.0.0.1:25565}" "''${2:-100}"
            '';

            # process-compose supervises the two processes instead of the
            # shell doing it: it gives dependency ordering (the proxy waits for
            # the game server's port), per-process restart policy, a readable
            # TUI with separated logs, and one Ctrl-C that actually stops
            # everything. GNU parallel gave none of that -- a crashed process
            # just vanished from an interleaved stream.
            dev = {
              deps = [ pkgs.process-compose pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                certs="$root/${certsDir}"
                if [ ! -f "$certs/root_ca.crt" ]; then
                  echo "no dev certificates yet; generating them" >&2
                  "${lib.getExe runners.certs}"
                fi
                cd "$root"
                # process-compose's own API port has to move with the game
                # ports, or a second checkout dies on 8080 before either process
                # starts.
                api_port="''${HYPERION_PC_PORT:-$(( 8080 + ''${HYPERION_PLAYER_PORT:-${toString proxyPort}} - ${toString proxyPort} ))}"
                echo "players: 0.0.0.0:''${HYPERION_PLAYER_PORT:-${toString proxyPort}} | game server: 127.0.0.1:''${HYPERION_SERVER_PORT:-${toString gameServerPort}}"
                exec process-compose --config ${processComposeConfig} --port "$api_port" "$@"
              '';
            };

            # `nix run .#dev` runs bedwars; this runs the same stack on smash.
            smash-dev = {
              deps = [ pkgs.process-compose pkgs.git ];
              text = ''
                export HYPERION_EVENT=smash
                exec "${lib.getExe runners.dev}" "$@"
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
              "git+https://github.com/nvzqz/divan#55ec68e31526c28c7825fa1bb884f326b619a879" =
                "sha256-xL0b6ZGmG4lhVcBjbBpobODZye6MAIr/gGBwMIrxmwM=";
              "git+https://github.com/andrewgazelka/Flecs-Rust?rev=252944dedbc80741b7cca30dea67c5be95638950#252944dedbc80741b7cca30dea67c5be95638950" =
                "sha256-3qUAXDHkeRFVfovZT+fW7VXW6aDteAiqCrLeCG/jd40=";
              "git+https://github.com/TestingPlant/valence?branch=feat-bytes#fb792dcb6669b64c5dc2366eb3d074b293def046" =
                "sha256-rpuJSz8KxEwG5qeT4HYVtTxHJ24nrYZJwDurv+mjPxM=";
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
              sync-minecraft-registry-data = minecraft.syncRegistryDataScript;
              extract-minecraft-protocol = minecraft.extractor;
              # `nix run .#minecraft-encode -- fixtures out.json` prints bytes
              # from the server's own codecs, so a new codec can be checked
              # against Mojang rather than against a reading of Mojang.
              minecraft-encode = minecraft.vanillaEncoder;
            });

          packages = {
            default = workspace.binaries.bedwars;
            inherit (workspace.binaries) bedwars hyperion-proxy rust-mc-bot;

            minecraft-server-jar = minecraft.serverJar;
            minecraft-data = minecraft.generatedData;
            minecraft-decompiled = minecraft.decompiledSources;
            minecraft-protocol = minecraft.protocolJson;
            minecraft-proto-rust = minecraft.generatedRust;
            minecraft-encoder-fixtures = minecraft.encoderFixtures;
            minecraft-registry-contents = minecraft.registryContents;
            minecraft-registry-data-rust = minecraft.generatedRegistryData;
          };

          # `nix flake check` builds every app, which is what proves each one
          # passes shellcheck and that its tools resolve.
          checks = scripts // {
            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-registry-data = minecraft.registryDataUpToDate;
            minecraft-encoder-fixtures = minecraft.fixturesUpToDate;
            minecraft-proto-json = minecraft.protocolJsonUpToDate;
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

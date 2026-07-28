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

            # Joins a running server with the real Minecraft client and says
            # whether a player reached the world.
            #
            # Everything else in this file reads the wire; this reads the game.
            # The distance between the two is not theoretical: the scripted
            # client passed for a week while every real client was dropped
            # during registry loading, because the scripted one does not load
            # registries. It cannot run in CI, since it needs a launcher, an
            # account and a GPU, so it is a command a person runs before saying
            # an address works.
            real-client = {
              deps = [ pkgs.bash ];
              text = ''exec ./tools/real-client-join.sh "$@"'';
            };

            # Boots the whole stack, drives a scripted 26.2 client through it,
            # and exits with that client's verdict.
            #
            # This is the only check that reads the wire the way a player does.
            # Every other gate reads the source: `nix run .#test` proves a
            # packet encodes to the bytes a test says it should, and stays green
            # while the server sends that packet under a number from a different
            # protocol version. A client is what notices.
            #
            # Ports come from the environment and default off the dev ports, so
            # a run does not fight a `nix run .#dev` open in another terminal.
            e2e = {
              deps = [
                pkgs.process-compose
                pkgs.git
                pkgs.python3
              ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                cd "$root"

                # The event and the client that drives it move together, so
                # `smash-e2e` sets both and this stays the bedwars gate. Two
                # apps rather than one with a flag, because the useful thing to
                # type is one word.
                export HYPERION_EVENT="''${HYPERION_EVENT:-bedwars}"
                read -ra client <<< "''${HYPERION_E2E_CLIENT:-tools/client-26.2.py --name e2e}"

                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + 1000)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + 1000)}}"
                player_port="$HYPERION_PLAYER_PORT"
                server_port="$HYPERION_SERVER_PORT"

                log="$(mktemp -t hyperion-e2e.XXXXXX)"
                echo "stack log: $log"

                "${lib.getExe runners.dev}" --tui=false >> "$log" 2>&1 &
                stack=$!
                # Killing the process group, not the pid: process-compose forks
                # cargo, which forks the server, and killing only $stack orphans
                # both so the next run dies on "address already in use".
                # shellcheck disable=SC2329  # run by the EXIT trap below
                cleanup() {
                  kill -- "-$stack" >> "$log" 2>&1 || kill "$stack" >> "$log" 2>&1 || true
                  wait "$stack" >> "$log" 2>&1 || true
                }
                trap cleanup EXIT

                # A cold run compiles two binaries, so the bound is generous. It
                # is a bound rather than a sleep because a warm run is ready in
                # seconds and should not pay for the cold one.
                # Both ports, not just the player one. The proxy binds its
                # listener immediately and retries the game server behind it, so
                # a probe that only checks the player port lets the client
                # connect while the game server is still compiling. The client
                # then dies on a read timeout that reads exactly like a protocol
                # bug, which cost an agent a full cycle to diagnose (ENG-10450).
                deadline=$(( SECONDS + 900 ))
                until python3 -c "
                import socket, sys
                for port in ($player_port, $server_port):
                    s = socket.socket()
                    s.settimeout(1)
                    if s.connect_ex(('127.0.0.1', port)) != 0:
                        sys.exit(1)
                sys.exit(0)
                "; do
                  if [ "$SECONDS" -ge "$deadline" ]; then
                    echo "stack never opened both 127.0.0.1:$player_port and 127.0.0.1:$server_port; tail of $log:" >&2
                    tail -40 "$log" >&2
                    exit 1
                  fi
                  if ! kill -0 "$stack" >> "$log" 2>&1; then
                    echo "stack exited before opening a port; tail of $log:" >&2
                    tail -40 "$log" >&2
                    exit 1
                  fi
                  sleep 2
                done

                echo "stack up on 127.0.0.1:$player_port ($HYPERION_EVENT)"
                # Not `exec`: replacing this shell would skip the EXIT trap and
                # orphan the stack, and the next run would die on "address
                # already in use".
                rc=0
                python3 "''${client[@]}" --host 127.0.0.1 --port "$player_port" "$@" || rc=$?

                # A client that finished its checks proves nothing if the server
                # died while it was reading. It has: the movement handler
                # aborted the process on the first step a player took
                # (hyperion#987), and the client saw only its own read timeout,
                # which it treats as a clean end of session. So ask the game
                # server directly whether it is still listening.
                if ! python3 -c "
                import socket, sys
                s = socket.socket()
                s.settimeout(2)
                sys.exit(0 if s.connect_ex(('127.0.0.1', $server_port)) == 0 else 1)
                "; then
                  echo "the game server stopped listening during the session; tail of $log:" >&2
                  tail -60 "$log" >&2
                  exit 1
                fi

                exit "$rc"
              '';
            };

            # The same gate on smash. bedwars is joinable with one client, so
            # `e2e` drives one; a smash match needs `min_players` of them at
            # once, which is why the client is a different program rather than
            # the same one with a flag.
            #
            # Ports default off the `e2e` ones rather than sharing them, so the
            # two gates can run side by side.
            smash-e2e = {
              deps = [
                pkgs.process-compose
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/smash-match.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + 2000)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + 2000)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that reads blocks rather
            # than positions.
            #
            # Separate from `smash-e2e` rather than folded into it because the
            # two answer different questions and fail for different reasons.
            # `smash-e2e` asks whether a match happens; this asks whether the
            # world the match happens in is the one the map files describe, and
            # whether the kill plane each of them declares is the height the
            # game actually kills at. Its ports default off `smash-e2e`'s again
            # so all three gates can run at once.
            smash-map-e2e = {
              deps = [
                pkgs.process-compose
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/smash-map-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + 3000)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + 3000)}}"
                exec "${lib.getExe runners.e2e}" "$@"
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
          # cargoUnit names a binary derivation after its cargo target, but
          # does not set `meta.mainProgram`, so `lib.getExe` guesses the
          # derivation name and misses. The NixOS modules below read
          # `meta.mainProgram` to build their ExecStart, so this is what makes
          # them work rather than a nicety.
          named =
            name: drv:
            drv.overrideAttrs (previous: {
              meta = (previous.meta or { }) // {
                mainProgram = name;
              };
            });

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
              sync-minecraft-tag-data = minecraft.syncTagDataScript;
              sync-minecraft-block-states = minecraft.syncBlockStatesScript;
              sync-minecraft-entity-types = minecraft.syncEntityTypesScript;
              extract-minecraft-protocol = minecraft.extractor;
              # `nix run .#minecraft-encode -- fixtures out.json` prints bytes
              # from the server's own codecs, so a new codec can be checked
              # against Mojang rather than against a reading of Mojang.
              minecraft-encode = minecraft.vanillaEncoder;
            });

          packages = {
            default = named "bedwars" workspace.binaries.bedwars;
            bedwars = named "bedwars" workspace.binaries.bedwars;
            hyperion-proxy = named "hyperion-proxy" workspace.binaries.hyperion-proxy;
            rust-mc-bot = named "rust-mc-bot" workspace.binaries.rust-mc-bot;

            minecraft-server-jar = minecraft.serverJar;
            minecraft-data = minecraft.generatedData;
            minecraft-decompiled = minecraft.decompiledSources;
            minecraft-protocol = minecraft.protocolJson;
            minecraft-proto-rust = minecraft.generatedRust;
            minecraft-encoder-fixtures = minecraft.encoderFixtures;
            minecraft-registry-contents = minecraft.registryContents;
            minecraft-tag-contents = minecraft.tagContents;
            minecraft-registry-data-rust = minecraft.generatedRegistryData;
            minecraft-tag-data-rust = minecraft.generatedTagData;
            minecraft-block-states-rust = minecraft.generatedBlockStates;
            minecraft-entity-types-rust = minecraft.generatedEntityTypes;
          };

          # `nix flake check` builds every app, which is what proves each one
          # passes shellcheck and that its tools resolve.
          checks = scripts // {
            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-registry-data = minecraft.registryDataUpToDate;
            minecraft-tag-data = minecraft.tagDataUpToDate;
            minecraft-tags-load = minecraft.tagsLoadForClient;
            minecraft-block-states = minecraft.blockStatesUpToDate;
            minecraft-entity-types = minecraft.entityTypesUpToDate;
            minecraft-encoder-fixtures = minecraft.fixturesUpToDate;
            minecraft-proto-json = minecraft.protocolJsonUpToDate;
            minecraft-protocol = minecraft.protocolJson;
          };

        };
    in
    {
      # A NixOS system that imports both modules and nothing else, built by
      # `nix flake check`. Without it the modules are only ever exercised by
      # whoever deploys them, and a typo in an option name is discovered on a
      # host rather than here.
      nixosConfigurations.module-smoke-test = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          self.nixosModules.game-server
          self.nixosModules.proxy
          (
            { lib, ... }:
            {
              boot.loader.grub.enable = false;
              fileSystems."/" = {
                device = "/dev/disk/by-label/nixos";
                fsType = "ext4";
              };
              system.stateVersion = "25.05";
              nixpkgs.hostPlatform = "x86_64-linux";

              # Paths that do not exist, which is the point: the modules must
              # not read them at evaluation time. A module that did would work
              # on the machine that has the certificates and fail everywhere
              # else.
              services.hyperion-game-server = {
                enable = true;
                pki = {
                  rootCaCert = "/var/lib/hyperion-pki/root_ca.crt";
                  cert = "/var/lib/hyperion-pki/game.crt";
                  privateKey = "/var/lib/hyperion-pki/game_private_key.pem";
                };
              };

              services.hyperion-proxy = {
                enable = true;
                gameServer = "hyperion-game.internal:35565";
                pki = {
                  rootCaCert = "/var/lib/hyperion-pki/root_ca.crt";
                  cert = "/var/lib/hyperion-pki/proxy.crt";
                  privateKey = "/var/lib/hyperion-pki/proxy_private_key.pem";
                };
              };
            }
          )
        ];
      };

      # NixOS modules for the two services, so a deployment imports them
      # rather than reimplementing a systemd unit per host. Not per system:
      # a module reads the host platform off the machine it lands on.
      nixosModules = {
        game-server = import ./nix/modules/game-server.nix {
          hyperionPackages = self.packages;
        };
        proxy = import ./nix/modules/proxy.nix {
          hyperionPackages = self.packages;
        };
      };

      apps = forAllSystems (system: (mkSystem system).apps);
      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        (mkSystem system).checks
        // {
          # Pins the command line the two modules build, so a renamed option
          # or a reordered argument is caught here rather than on a host.
          #
          # The store paths are context-stripped on purpose. Keeping the
          # context would make this check depend on the binaries, and a module
          # check should run everywhere in seconds rather than compile a
          # server. What a module gets wrong is the spelling of a flag or the
          # order of the arguments, and that is what this reads.
          nixos-modules =
            let
              units = self.nixosConfigurations.module-smoke-test.config.systemd.services;
              argv = unit: builtins.unsafeDiscardStringContext units.${unit}.serviceConfig.ExecStart;
            in
            pkgs.runCommand "hyperion-nixos-modules" { } ''
              cat > argv <<'ARGV'
              ${argv "hyperion-game-server"}
              ${argv "hyperion-proxy"}
              ARGV

              expect() {
                grep -qF -- "$1" argv || {
                  echo "hyperion NixOS module argv lost: $1" >&2
                  echo "what the modules built:" >&2
                  cat argv >&2
                  exit 1
                }
              }

              expect "/bin/bedwars --ip :: --port 35565"
              expect "/bin/hyperion-proxy '[::]:25565' --server hyperion-game.internal:35565"
              expect "--root-ca-cert /var/lib/hyperion-pki/root_ca.crt --cert /var/lib/hyperion-pki/game.crt --private-key /var/lib/hyperion-pki/game_private_key.pem"
              expect "--root-ca-cert /var/lib/hyperion-pki/root_ca.crt --cert /var/lib/hyperion-pki/proxy.crt --private-key /var/lib/hyperion-pki/proxy_private_key.pem"

              touch $out
            '';
        }
      );
      devShells = forAllSystems (system: (mkSystem system).devShells);
      packages = forAllSystems (system: (mkSystem system).packages);
    };
}

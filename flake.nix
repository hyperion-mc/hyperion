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

          # Split out because it is the only pipeline that runs the server
          # rather than reading data out of it, and it is the only one whose
          # output is checked by a Rust test rather than compiled into one.
          differential = import ./nix/differential.nix {
            pkgs = minecraftPkgs;
            inherit (minecraft) jdk serverClasspath pin;
          };

          # Styling is a field on a component, never characters in a string.
          # See the file: the type covers the seam, this covers the one
          # spelling a type cannot.
          textGate = import ./nix/text.nix {
            inherit pkgs;
            sources = {
              smashSource = ./events/smash/src;
              protoSource = ./crates/hyperion-minecraft-proto/src;
            };
          };

          # One harness behind both the `nix run` gates and the sandboxed
          # checks, so the two cannot drift.
          e2e = import ./nix/e2e.nix {
            inherit pkgs lib fileDescriptors;
            sources = {
              root = ./.;
              tools = ./tools;
              protoSource = ./crates/hyperion-minecraft-proto/src;
              # The scripted clients resolve registry ids to names through
              # this, the same file build.rs reads.
              protocolJson = ./crates/hyperion-minecraft-proto/protocol.json;
              kitSkins = ./events/smash/skins;
              genmap = ./crates/hyperion-genmap/src/lib.rs;
            };
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

          # Signature checking, and nothing else, so this stays a small closure
          # a `nix flake check` can afford.
          kitSkinPython = pkgs.python3.withPackages (python: [ python.cryptography ]);

          # The files the skin check reads: the payloads, Mojang's keys, and the
          # kit sources that declare which payload is whose. Narrow on purpose,
          # so editing an unrelated Rust file does not rebuild the check.
          kitSkinSource = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./nix/verify-kit-skins.py
              ./events/smash/skins
              ./events/smash/src/module/kits
            ];
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

            # Every kit's skin has to be one the client will show to other
            # players, and that means Mojang-signed: an unsigned `textures`
            # property renders for its wearer and for nobody else. See
            # `events/smash/skins/README.md` for where the client enforces it.
            check-kit-skins = {
              deps = [
                kitSkinPython
                pkgs.git
              ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                exec python3 "$root/nix/verify-kit-skins.py"
              '';
            };

            lint.text = ''cargo clippy ${clippyArgs} -- -D warnings'';

            lint-fix.text = ''
              cargo clippy --fix --allow-dirty --allow-staged ${clippyArgs} -- -D warnings
            '';

            test = {
              deps = [ pkgs.cargo-nextest ];
              text = ''cargo nextest run "$@"'';
            };

            # How much of the game the tests actually check, as a number.
            #
            # A mutant is a deliberate change to the source: a `<` flipped to
            # `<=`, a function body replaced with a constant. A mutant the suite
            # still passes with is a line the tests execute without checking, so
            # the surviving count measures verification rather than coverage --
            # `nix run .#test` staying green while a mutant lives is exactly
            # what this catches and what a coverage percentage cannot.
            #
            # Scope and exclusions live in `.cargo/mutants.toml`, next to the
            # reasons for them. The budget is here because it is a policy
            # question rather than a configuration one.
            #
            # Raising MUTANT_BUDGET is a change to how much of the game is
            # checked, so it needs the same argument in a pull request that
            # deleting a test would. A mutant genuinely not worth killing gets
            # an exclusion in `mutants.toml` with a reason beside it instead.
            #
            # Roughly fifteen minutes on four cores; not part of `nix flake
            # check`, which only builds this script.
            mutants = {
              deps = [
                pkgs.cargo-mutants
                pkgs.cargo-nextest
                pkgs.coreutils
                pkgs.git
              ];
              text = ''
                budget="''${MUTANT_BUDGET:-0}"
                root="$(git rev-parse --show-toplevel)"
                cd "$root"

                out="''${MUTANT_OUTPUT:-$root/target/mutants}"
                rc=0
                cargo mutants --test-tool nextest -j "$(nproc)" --output "$out" "$@" || rc=$?
                # 0 is a clean sweep and 2 is "some survived", which is the
                # normal case and is judged against the budget below. Anything
                # else is cargo-mutants itself failing, and that is not a
                # verdict on the tests.
                case "$rc" in
                  0 | 2) ;;
                  *)
                    echo "cargo-mutants exited $rc, which is a tool failure rather than a result" >&2
                    exit "$rc"
                    ;;
                esac

                missed="$(wc -l < "$out/mutants.out/missed.txt" | tr -d ' ')"
                caught="$(wc -l < "$out/mutants.out/caught.txt" | tr -d ' ')"
                echo "mutants: $caught killed, $missed survived (budget $budget)"

                if [ "$missed" -gt "$budget" ]; then
                  echo "" >&2
                  echo "FAIL: $missed mutants survived, which is more than the budget of $budget." >&2
                  echo "Each line below is a change to the source that every test still passes with." >&2
                  echo "" >&2
                  cat "$out/mutants.out/missed.txt" >&2
                  exit 1
                fi

                if [ "$missed" -lt "$budget" ]; then
                  echo "The budget is now loose by $((budget - missed)). Lower MUTANT_BUDGET in flake.nix to hold the ground."
                fi
              '';
            };

            # The unbounded version of the decoder fuzz that `nix run .#test`
            # runs a fixed slice of.
            #
            # The gate runs four thousand cases from seed zero, the same four
            # thousand every time, because a check that fuzzes differently on
            # every run fails for somebody else on a case they cannot get back.
            # This walks the seed base forward instead and does not stop, which
            # is the same generator searching rather than checking. Leave it
            # running; a failure prints the seed, and the seed is enough to
            # reproduce the case in the one-second version.
            fuzz = {
              deps = [
                pkgs.cargo-nextest
                pkgs.git
              ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                cd "$root"

                export HYPERION_FUZZ_SEEDS="''${HYPERION_FUZZ_SEEDS:-4096}"
                export HYPERION_FUZZ_CASES="''${HYPERION_FUZZ_CASES:-256}"
                base="''${HYPERION_FUZZ_SEED_BASE:-0}"
                batch=$(( HYPERION_FUZZ_SEEDS ))

                echo "fuzzing the packet decoder from seed $base, $batch seeds a round; ctrl-c to stop"
                while :; do
                  export HYPERION_FUZZ_SEED_BASE="$base"
                  # One line: a backslash continuation is literal inside a Nix
                  # indented string and reaches the shell as a stray argument.
                  if ! cargo nextest run -p hyperion-minecraft-proto --no-capture -E 'binary(decode_fuzz)' "$@"; then
                    echo "" >&2
                    echo "a case in seeds $base..$(( base + batch )) broke the decoder." >&2
                    echo "reproduce it with the gate-sized run:" >&2
                    echo "  HYPERION_FUZZ_SEED_BASE=$base HYPERION_FUZZ_SEEDS=$batch HYPERION_FUZZ_CASES=$HYPERION_FUZZ_CASES nix run .#test -- -p hyperion-minecraft-proto" >&2
                    exit 1
                  fi
                  base=$(( base + batch ))
                done
              '';
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

          # How far each end to end gate's default ports sit above those two.
          #
          # One attribute set rather than a number written into each gate's
          # script, because two gates claiming one offset is invisible where the
          # numbers are apart: `completions-e2e` and `smash-selector-e2e` both
          # said 4000 for weeks (ENG-10834), and on darwin, where a build shares
          # the host's loopback, the loser of that race fails with "address
          # already in use" from a gate it has nothing to do with. Collected
          # here, `e2ePortsDistinct` below can say so at eval time.
          e2eOffsets = {
            e2e = 1000;
            smash-e2e = 2000;
            smash-map-e2e = 3000;
            smash-selector-e2e = 4000;
            smash-identity-e2e = 5000;
            completions-e2e = 6000;
            smash-hotbar-e2e = 7000;
            smash-hud-e2e = 8000;
          };

          # The lobby `smash-e2e` runs against, which is deliberately not the
          # one the product ships.
          #
          # The gate's ability sweep changes kits, and a committed match
          # refuses to, so the sweep needs a roster the lobby will not start
          # on. That roster also needs at least two players in it, because
          # `hurts_target` and `heals_caster` are claims about a second body.
          # Production runs 2/4 so two people can start a game, and under 2/4
          # no roster is both things at once. So the gate states the lobby it
          # needs rather than inferring one, and `smash-match.py` checks it got
          # it: the run fails if the lobby leaves the hub mid-sweep. Before
          # this, the harness had `min_players - 1` written into it and the
          # numbers moved out from under it (#1019).
          #
          # Both thresholds and not only the minimum: `full_players` is what
          # decides the three-quarters band, which is the one that actually
          # fired. At 2/4 three players satisfy `3 * 4 >= 4 * 3` and the sweep
          # died 0.6 seconds in. These are the pre-#1019 numbers, under which
          # three players start nothing even by the old band order, so this
          # gate does not depend on the `countdown_for` fix that shipped
          # alongside it.
          smashGateLobby = {
            sweepClients = 3;
            env = {
              SMASH_MIN_PLAYERS = 4;
              SMASH_FULL_PLAYERS = 8;
            };
          };

          # `env` as shell, for the gates that are scripts rather than checks.
          exportsFor =
            env:
            lib.concatStringsSep "\n" (
              lib.mapAttrsToList (name: value: "export ${name}=${toString value}") env
            );

          # Two gates on one offset, as a build failure rather than as a race.
          #
          # An eval-time check and not a runtime one: the failure it catches is a
          # number, it costs nothing to look at, and the alternative is finding
          # out from whichever unlucky gate lost the port. The names are reported
          # rather than only the count, because "two gates collide" is not
          # actionable and "these two gates collide" is.
          e2ePortsDistinct =
            let
              offsets = lib.attrValues e2eOffsets;
              duplicated = lib.filter (
                offset: lib.length (lib.filter (other: other == offset) offsets) > 1
              ) offsets;
              colliding = lib.filter (name: lib.elem e2eOffsets.${name} duplicated) (
                lib.attrNames e2eOffsets
              );
            in
            pkgs.runCommand "hyperion-e2e-ports-distinct" { } (
              if duplicated == [ ] then
                ''
                  echo "ok: ${toString (lib.length offsets)} end to end gates, ${
                    toString (lib.length offsets)
                  } distinct port offsets"
                  touch "$out"
                ''
              else
                ''
                  echo "FAIL: these gates claim the same port offset: ${
                    lib.concatStringsSep ", " colliding
                  }" >&2
                  echo "Every gate in e2eOffsets needs its own number, or two of" >&2
                  echo "them running at once fight over one port and the loser" >&2
                  echo "fails with address already in use." >&2
                  exit 1
                ''
            );

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
                pkgs.git
                e2e.driver
              ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                cd "$root"

                # The event and the client that drives it move together, so
                # `smash-e2e` sets both and this stays the bedwars gate. Two
                # apps rather than one with a flag, because the useful thing to
                # type is one word.
                event="''${HYPERION_EVENT:-bedwars}"
                profile="''${HYPERION_PROFILE:-dev}"

                # cargo, not the store: what a person debugging wants is the
                # code in their working tree, rebuilt incrementally. The check
                # of the same name hands the same driver two store paths, and
                # that is the only difference between them.
                export HYPERION_E2E_GAME_SERVER="cargo run --profile $profile -p $event --"
                export HYPERION_E2E_PROXY="cargo run --profile $profile --bin hyperion-proxy --"
                export HYPERION_E2E_CLIENT="''${HYPERION_E2E_CLIENT:-tools/client-26.2.py --name e2e}"
                # Certificates from the store rather than `nix run .#certs`, so
                # a fresh clone runs this without a setup step first.
                export HYPERION_E2E_CERTS="${e2e.certs}"

                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.e2e)}}"

                exec hyperion-e2e-driver "$@"
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
              deps = [ pkgs.git ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT="tools/smash-match.py --sweep-clients ${toString smashGateLobby.sweepClients}"
                ${exportsFor smashGateLobby.env}
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that clicks the hub's kit
            # podiums instead of playing the match.
            #
            # Separate from `smash-e2e` because it answers the question that
            # comes before it. `smash-e2e` needs kits to already be picked and
            # picks them by typing `/kit`; this one asks whether a player who
            # never types anything can pick a mob by right-clicking it, and
            # whether the game says so when the mob is somebody else's. Its
            # ports default off the others again so every gate can run at once.
            smash-selector-e2e = {
              deps = [ pkgs.git ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/smash-selector.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-selector-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-selector-e2e)}}"
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
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-map-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-map-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that types rather than
            # walks.
            #
            # `smash-e2e` and `smash-map-e2e` both prove things about a world.
            # This one never leaves the hub: it reads the command graph the
            # server sends on join and then presses tab, which is the whole of
            # the completion path and touches nothing else.
            #
            # +6000 and not +4000, which is what it claimed until ENG-10834:
            # `smash-selector-e2e` claims that offset too, so running the two
            # side by side had one of them lose the port to the other. On darwin
            # a build shares the host's loopback, so the loser fails with
            # "address already in use" from a gate it has nothing to do with.
            # Every offset in this file is now distinct, which the check below
            # is what says.
            completions-e2e = {
              deps = [ pkgs.git ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/completions-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.completions-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.completions-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that asks who a player is
            # rather than what they can do: two connections under one IGN,
            # whether a dig is refused, and whether the profile the other
            # player receives carries the kit's mob. Ports default off
            # `completions-e2e`'s so all five gates can run side by side.
            smash-identity-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/identity-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-identity-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-identity-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that reads its own
            # inventory rather than the world.
            #
            # Separate from `smash-e2e` because it needs no match at all: one
            # client stands in the hub, changes kit fifteen times and reads the
            # bar it is handed each time. `smash-e2e` would eventually notice a
            # kit whose abilities were unreachable, but only by failing to fire
            # one; this fails on the layout itself and names the kit. Ports
            # default off `smash-identity-e2e`'s so all six gates can run side
            # by side.
            smash-hotbar-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/hotbar-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-hotbar-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-hotbar-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The same gate again, driving a client that reads the screen rather
            # than the world: the experience bar as an ability recharges, the
            # boss bar through every phase, and the titles a match punctuates
            # itself with. Three of those packets were sent zero times by this
            # server before the HUD landed, so nothing else here can tell a
            # regression from the way it always was.
            #
            # Its ports come from `e2eOffsets`, which is also where
            # `smash-hotbar-e2e` has just been moved to: it was written with a
            # literal 6000, which is `completions-e2e`'s, and a literal is
            # exactly what `e2ePortsDistinct` cannot see. Two gates on one
            # offset is a race for a socket on darwin, and the guard only works
            # if every gate is in the table it reads.
            smash-hud-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/hud-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-hud-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-hud-e2e)}}"
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

          # Named once and used by both `packages` and the sandboxed checks, so
          # a gate runs the same binary the flake publishes rather than a second
          # build of it.
          gameBinaries = {
            bedwars = named "bedwars" workspace.binaries.bedwars;
            smash = named "smash" workspace.binaries.smash;
            hyperion-proxy = named "hyperion-proxy" workspace.binaries.hyperion-proxy;
          };

          # `nix flake check` builds every app, which is what proves each one
          # passes shellcheck and that its tools resolve. What CI enforces of
          # this set, and the named exceptions, live in nix/ci/flake-gate.nix.
          baseChecks = scripts // {
            # The two gates that read the wire the way a player does, as
            # derivations: nix builds the binaries, the sandbox boots them on
            # loopback, and the scripted client's verdict is the build result.
            # Every other check reads the source. `nix run .#test` proves a
            # packet encodes to the bytes a test says it should, and stays
            # green while the server sends that packet under a number from a
            # different protocol version. A client is what notices, and until
            # these existed nothing in `nix flake check` ran one.
            e2e = e2e.mkCheck {
              name = "hyperion-e2e";
              gameServer = gameBinaries.bedwars;
              proxy = gameBinaries.hyperion-proxy;
              client = "client-26.2.py";
              clientArgs = [ "--name" "e2e" ];
              # bedwars still downloads its world at boot, so the sandbox has
              # to hand it one. smash below needs nothing: its arenas are
              # `include_str!` of files this repository owns.
              needsGenMap = true;
            };

            smash-e2e = e2e.mkCheck {
              name = "hyperion-smash-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "smash-match.py";
              clientArgs = [
                "--sweep-clients"
                (toString smashGateLobby.sweepClients)
              ];
              serverEnv = smashGateLobby.env;
              # A match is four clients playing for up to five minutes, so the
              # cap is the client's own budget plus room to boot and report.
              timeout = 480;
            };

            # Whether a player who types a slash sees anything. Two protocol
            # mechanisms answer that, the command graph and the suggestion
            # request, and neither is visible to a Rust test: the graph is a
            # packet nobody sends unless a client joins, and a suggestion is a
            # reply to a packet only a client sends.
            completions-e2e = e2e.mkCheck {
              name = "hyperion-completions-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "completions-check.py";
            };

            # The kit selector, driven by right-clicks on the podium mobs
            # rather than by `/kit`. Shorter than the match gate because it
            # never plays one: the longest thing in it is the countdown a full
            # lobby runs.
            smash-selector-e2e = e2e.mkCheck {
              name = "hyperion-smash-selector-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "smash-selector.py";
              timeout = 420;
            };

            # Identity, permissions and appearance, on a real connection.
            # Separate from `smash-e2e` because it fails for entirely different
            # reasons: that one asks whether a match happens, this one asks who
            # the people in it are.
            smash-identity-e2e = e2e.mkCheck {
              name = "hyperion-smash-identity-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "identity-check.py";
              timeout = 300;
            };

            # Which key each kit's abilities land on, read off the inventory
            # packets a real client receives. The failure it catches is silent
            # everywhere else: an ability bound one key to the right of where a
            # hand rests is present, correct and unreachable.
            smash-hotbar-e2e = e2e.mkCheck {
              name = "hyperion-smash-hotbar-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "hotbar-check.py";
              # Fifteen kit changes in a hub that never starts a match, so this
              # is the shortest of the smash gates.
              timeout = 300;
            };

            # The screen, on a real connection. Eight clients rather than four,
            # because `full_players` is what makes the lobby run its ten second
            # countdown instead of its sixty second one, and the countdown is
            # half of what this reads.
            smash-hud-e2e = e2e.mkCheck {
              name = "hyperion-smash-hud-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "hud-check.py";
              timeout = 420;
            };

            # `checks.e2e` above took the names the two app wrappers used to
            # hold, and those wrappers still have to pass shellcheck.
            e2e-app = scripts.e2e;
            smash-e2e-app = scripts.smash-e2e;
            completions-e2e-app = scripts.completions-e2e;
            smash-selector-e2e-app = scripts.smash-selector-e2e;
            smash-identity-e2e-app = scripts.smash-identity-e2e;
            smash-hotbar-e2e-app = scripts.smash-hotbar-e2e;
            smash-hud-e2e-app = scripts.smash-hud-e2e;

            # Every kit's skin, checked offline against Mojang's committed
            # public keys. A derivation rather than only an app, because the
            # failure it catches is silent: an unsigned payload sends cleanly,
            # renders for its wearer, and leaves everyone else seeing Steve.
            kit-skins-signed =
              pkgs.runCommand "hyperion-kit-skins-signed"
                {
                  nativeBuildInputs = [ kitSkinPython ];
                }
                ''
                  python3 ${kitSkinSource}/nix/verify-kit-skins.py | tee "$out"
                '';

            # The pinned world URL still has to be the one the server asks for.
            genmap-url-pinned = e2e.genMapUrlPinned;

            # No two end to end gates may claim one port. See `e2eOffsets`.
            e2e-ports-distinct = e2ePortsDistinct;

            # A colour reaches a client as a component field or not at all.
            smash-text-no-legacy-formatting = textGate;

            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-proto-coverage = minecraft.coverageRatchet;
            tool-paths = minecraft.toolPaths;
            minecraft-registry-data = minecraft.registryDataUpToDate;
            minecraft-tag-data = minecraft.tagDataUpToDate;
            minecraft-tags-load = minecraft.tagsLoadForClient;
            minecraft-block-states = minecraft.blockStatesUpToDate;
            minecraft-encoder-fixtures = minecraft.fixturesUpToDate;
            minecraft-proto-json = minecraft.protocolJsonUpToDate;
            minecraft-protocol = minecraft.protocolJson;
            # A jar bump that changes vanilla's physics must not leave the
            # committed traces behind, or the Rust test passes against a
            # recording of a server nobody runs any more.
            differential-traces = differential.tracesUpToDate;

            # Pins the command line the two modules build, so a renamed option
            # or a reordered argument is caught here rather than on a host.
            #
            # The store paths are context-stripped on purpose. Keeping the
            # context would make this check depend on the binaries, and a
            # module check should run everywhere in seconds rather than
            # compile a server. What a module gets wrong is the spelling of a
            # flag or the order of the arguments, and that is what this reads.
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
          };

          # Self-referential on purpose: the gate is told every name in
          # `checks`, including its own, so a check added later is enforced
          # without anyone editing a list. `builtins.attrNames` reads the keys
          # of an attrset without forcing its values, which is what keeps this
          # from looping.
          checks = baseChecks // {
            flake-gate = import ./nix/ci/flake-gate.nix {
              inherit lib system;
              inherit (pkgs) writeShellApplication;
              names = builtins.attrNames checks;
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
              # What CI runs. A contributor can run the same one command and
              # see the same verdict, which is what stops the two drifting.
              inherit (checks) flake-gate;
              update-minecraft-data = minecraft.updateScript;
              sync-minecraft-proto = minecraft.syncScript;
              sync-minecraft-registry-data = minecraft.syncRegistryDataScript;
              sync-minecraft-tag-data = minecraft.syncTagDataScript;
              sync-minecraft-block-states = minecraft.syncBlockStatesScript;
              # Re-records the golden traces `crates/hyperion/tests/differential.rs`
              # compares against. See docs/differential-testing.md.
              record-differential-traces = differential.syncScript;
              extract-minecraft-protocol = minecraft.extractor;
              check-minecraft-proto-coverage = minecraft.coverageChecker;
              # `nix run .#minecraft-encode -- fixtures out.json` prints bytes
              # from the server's own codecs, so a new codec can be checked
              # against Mojang rather than against a reading of Mojang.
              minecraft-encode = minecraft.vanillaEncoder;
            });

          packages = {
            default = gameBinaries.bedwars;
            inherit (gameBinaries) bedwars smash hyperion-proxy;
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
            differential-recorder = differential.recorder;
            differential-traces = differential.recordedTraces;
            minecraft-block-states-rust = minecraft.generatedBlockStates;
          };

          # What CI enforces of this set is nix/ci/flake-gate.nix.
          inherit checks;

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
      checks = forAllSystems (system: (mkSystem system).checks);
      devShells = forAllSystems (system: (mkSystem system).devShells);
      packages = forAllSystems (system: (mkSystem system).packages);
    };
}

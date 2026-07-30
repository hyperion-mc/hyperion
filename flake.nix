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

          # The client scripts, plus the rule they are held to. Narrow on
          # purpose: a check whose input is the whole tree reruns on every
          # commit and stops being cheap.
          wireAssertionSource = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./nix/verify-wire-assertions.py
              ./tools
            ];
          };

          # The files the skin check reads: the payloads, Mojang's keys, and the
          # kit sources that declare which payload is whose. Narrow on purpose,
          # so editing an unrelated Rust file does not rebuild the check.
          kitSkinSource = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./nix/verify-kit-skins.py
              ./nix/verify-kit-skin-images.py
              ./events/smash/skins
              ./events/smash/src/module/kits
            ];
          };

          # Every kit's skin image, fetched from the url its signed payload names
          # and pinned by hash in textures.lock.json. Fixed-output so the image
          # gate stays offline and store-cached: the fetch happens once, here,
          # not inside the sandboxed check and not on every run.
          kitSkinImages =
            let
              lock = lib.importJSON ./events/smash/skins/textures.lock.json;
            in
            pkgs.runCommand "hyperion-kit-skin-images" { } (
              lib.concatStringsSep "\n" (
                [ "mkdir -p $out" ]
                ++ lib.mapAttrsToList (
                  mob: entry:
                  "cp ${pkgs.fetchurl { inherit (entry) url; hash = entry.sha256; }} $out/${mob}.png"
                ) lock
              )
            );

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

            # Repin every kit's skin image by hash after a skin changes. Impure
            # (it reaches Mojang's texture host), which is why it is a command
            # and not a check; the check that reads its output is offline.
            sync-kit-skin-textures = {
              deps = [
                pkgs.python3
                pkgs.nix
                pkgs.git
              ];
              text = ''
                root="$(git rev-parse --show-toplevel)"
                exec python3 "$root/nix/sync-kit-skin-textures.py"
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
            smash-skin-e2e = 8500;
            smash-bow-e2e = 8700;
            bedwars-bow-e2e = 9000;
            # Not 10000: gameServerPort - proxyPort == 10000, so an offset of
            # 10000 aliases this gate's player port onto the base game-server
            # port (35565), which a running dev server or `nix run .#run` holds.
            # Kept under 10000 so every player port stays below that base, the
            # invariant the offsets 1000..9000 already rely on. See ENG-10933.
            oob-move-e2e = 9500;
          };

          # The accounts `smash-hud-e2e` runs its `Admin` commands as.
          #
          # `/serverload` is `Admin` and nothing lets a client give itself a
          # group, so the three clients that need it are named to the server as
          # configuration before it starts, exactly as an operator would name
          # their own administrators (`hyperion_permission::seed`). The gate
          # used to promote itself with `/perms set`, which worked only while
          # that command was gated at `Normal`: ENG-10871.
          #
          # The ids and the configuration naming them come off one list,
          # because a client logging in under an id nobody configured is not an
          # administrator and nothing says so: the failure surfaces much later
          # as `/serverload` doing nothing.
          hudAdmins =
            let
              uuids = [
                "0be9a1d4-5c00-4e6a-9d21-6a5a1e0000a1"
                "0be9a1d4-5c00-4e6a-9d21-6a5a1e0000a2"
                "0be9a1d4-5c00-4e6a-9d21-6a5a1e0000a3"
              ];
            in
            {
              env.HYPERION_PERMISSIONS = lib.concatMapStringsSep "," (uuid: "${uuid}=Admin") uuids;
              clientArgs = lib.concatMap (uuid: [
                "--admin-uuid"
                uuid
              ]) uuids;
            };

          # The lobby `smash-e2e` and `smash-selector-e2e` run against, which
          # is deliberately not the one the product ships.
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
          #
          # `smash-selector-e2e` needs the same thing for the same reason: its
          # hub checks run on three clients, which #1019 also put above the
          # threshold, and its last check fills the lobby on purpose and so
          # needs to know what full is.
          smashGateLobby = {
            sweepClients = 3;
            env = {
              SMASH_MIN_PLAYERS = 4;
              SMASH_FULL_PLAYERS = 8;
            };
          };

          # The roster that fills the gate's lobby, read off the threshold
          # rather than written twice.
          smashGateFullClients = toString smashGateLobby.env.SMASH_FULL_PLAYERS;

          # `env` as shell, for the gates that are scripts rather than checks.
          exportsFor =
            env:
            lib.concatStringsSep "\n" (
              lib.mapAttrsToList (
                name: value: "export ${name}=${lib.escapeShellArg (toString value)}"
              ) env
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

          # Every directory under events/ is a game server crate, so the set of
          # events is read from the tree rather than listed here. A new event
          # directory becomes a `nix run .#<event>` app with no flake edit.
          events = lib.attrNames (
            lib.filterAttrs (_: type: type == "directory") (builtins.readDir ./events)
          );

          # One process-compose definition, parameterized by event name: the
          # game server runs that event's crate and nothing selects it at run
          # time, so `nix run .#smash` and `nix run .#bedwars` are the same
          # stack with the event fixed rather than one generic stack behind an
          # environment variable.
          #
          # Generated rather than committed as YAML: the ports and cert paths
          # then have one source of truth shared with the standalone apps above.
          # Commands are single-line: a backslash continuation is literal in a
          # Nix indented string, survives into the YAML, and reaches the shell
          # as a stray argument rather than a line join.
          mkProcessComposeConfig = event: (pkgs.formats.yaml { }).generate "process-compose-${event}.yaml" {
            version = "0.5";
            processes = {
              game-server = {
                command = "cargo run --profile \"$\{HYPERION_PROFILE:-dev}\" -p ${event} -- --ip 0.0.0.0 --port \"$\{HYPERION_SERVER_PORT:-${toString gameServerPort}}\" --root-ca-cert ${certsDir}/root_ca.crt --cert ${certsDir}/server.crt --private-key ${certsDir}/server_private_key.pem";
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

          # Each event gets a `nix run .#<event>` app that boots that same stack
          # with the event fixed: certificates first if missing, then the game
          # server and proxy under process-compose. One generator over the
          # events list rather than a copy per event, so a new event directory
          # gets a run app for free and there is nothing to keep in sync.
          mkDevStack = event: {
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
              echo "event: ${event} | players: 0.0.0.0:''${HYPERION_PLAYER_PORT:-${toString proxyPort}} | game server: 127.0.0.1:''${HYPERION_SERVER_PORT:-${toString gameServerPort}}"
              exec process-compose --config ${mkProcessComposeConfig event} --port "$api_port" "$@"
            '';
          };

          eventDevStacks = lib.genAttrs events mkDevStack;

          runners = lib.mapAttrs mkScript (eventDevStacks // {
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

            bots.text = ''
              ulimit -Sn ${fileDescriptors}
              exec cargo run --release -p rust-mc-bot -- \
                "''${1:-127.0.0.1:25565}" "''${2:-100}"
            '';

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
            # a run does not fight a `nix run .#bedwars` stack open in another terminal.
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
                export HYPERION_E2E_CLIENT="tools/smash-selector.py --full-clients ${smashGateFullClients}"
                ${exportsFor smashGateLobby.env}
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

            # What every other player sees of a player who put on a kit: the
            # committed skin, its Mojang signature, and the skin-overlay mask
            # with the hat bit set. Separate from `smash-identity-e2e` because
            # it reads the entity metadata as well as the tab list, and it is
            # the regression gate for the skin and hat bugs.
            smash-skin-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/skin-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-skin-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-skin-e2e)}}"
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
                export HYPERION_E2E_CLIENT="tools/hud-check.py ${lib.escapeShellArgs hudAdmins.clientArgs}"
                ${exportsFor hudAdmins.env}
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-hud-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-hud-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # A denial-of-service gate: a single connected client sends the
            # movement packets ENG-10914 proved could crash the whole server --
            # a finite coordinate past the i16 chunk range, then NaN and +inf --
            # and the run passes only if the server refuses them and keeps
            # ticking. Drives the proven client-26.2.py + bedwars pair the `e2e`
            # gate uses, so reaching play is not in question; the crash is in the
            # shared movement handler, so the event it runs against is incidental.
            oob-move-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=bedwars
                export HYPERION_E2E_CLIENT="tools/client-26.2.py --name oob --oob-move"
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.oob-move-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.oob-move-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The bow, on bedwars, read off the wire by the client that fired it.
            #
            # The only gate here that is not a smash gate, because the bow is
            # not a smash feature: Super Smash Mobs hands a Skeleton a bow as
            # the *icon* for a charged ability, and `smash-e2e` already proves
            # that path. This is the vanilla weapon -- nock, spend an arrow,
            # launch it at a speed the draw decides -- which only bedwars has,
            # and bedwars is what `nix run .#bedwars` and `packages.default` build.
            #
            # It asserts the launch velocity out of `ClientboundAddEntity`
            # rather than watching where the arrow lands, because since 26.2
            # that packet carries the velocity and a number is a far sharper
            # claim than a trajectory. The bug it was written for shipped a
            # fully drawn bow at 3.6 blocks a tick instead of 3.0 and no gate
            # noticed, because no gate looked.
            bedwars-bow-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_E2E_CLIENT=tools/bow-check.py
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.bedwars-bow-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.bedwars-bow-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

            # The smash bow read off the wire: one client takes the Skeleton kit
            # in the hub, draws Barrage at two lengths and releases. Proves the
            # arrow leaves the eye, its heading is `look_angles(velocity)`, it is
            # broadcast per tick so it flies, and a longer draw is faster. The
            # smash mirror of `bedwars-bow-e2e`.
            smash-bow-e2e = {
              deps = [
                pkgs.git
                pkgs.python3
              ];
              text = ''
                export HYPERION_EVENT=smash
                export HYPERION_E2E_CLIENT=tools/smash-bow-check.py
                ${exportsFor smashGateLobby.env}
                export HYPERION_PLAYER_PORT="''${HYPERION_PLAYER_PORT:-${toString (proxyPort + e2eOffsets.smash-bow-e2e)}}"
                export HYPERION_SERVER_PORT="''${HYPERION_SERVER_PORT:-${toString (gameServerPort + e2eOffsets.smash-bow-e2e)}}"
                exec "${lib.getExe runners.e2e}" "$@"
              '';
            };

          });

          scripts = checkScripts // runners // { inherit ci; };

          cargoUnit = index.lib.cargoUnitExternal {
            inherit pkgs rustToolchain;
          };

          # Shared across the release workspace below and the dev-profile one
          # the boot gate uses, so the source, lock and dependency hashes have
          # one definition rather than two that drift.
          workspaceArgs = {
            src = ./.;
            workspaceRoot = ./.;
            cargoLock = ./Cargo.lock;

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
              "git+https://github.com/andrewgazelka/Flecs-Rust?rev=f09dc5308d00c6a88c82b1195334b6ed2b2d2868#f09dc5308d00c6a88c82b1195334b6ed2b2d2868" =
                "sha256-DlMOSY7NyoPoR8w4yswm3O97BegcNWcLl/fW3wOAmRs=";
              "git+https://github.com/TestingPlant/valence?branch=feat-bytes#fb792dcb6669b64c5dc2366eb3d074b293def046" =
                "sha256-rpuJSz8KxEwG5qeT4HYVtTxHJ24nrYZJwDurv+mjPxM=";
            };
          };

          workspace = cargoUnit.buildWorkspace (
            workspaceArgs
            // {
              pname = "hyperion";
              cargoArgs = [ "--workspace" ];
            }
          );

          # The same game servers, compiled with `debug_assertions` on -- the
          # dev profile `nix run .#smash` runs and the operator actually plays.
          # This exists only for the boot gate: a release build compiles out the
          # flecs "component is not registered" assert, so the release binaries
          # above boot cleanly even when a singleton is set but never registered
          # (ENG-11000). Booting these catches that class before it reaches a
          # host. Unoptimised on purpose -- the dev profile is both the fastest
          # to compile and the one under test.
          devWorkspace = cargoUnit.buildWorkspace (
            workspaceArgs
            // {
              pname = "hyperion-dev";
              cargoArgs = [
                "-p"
                "smash"
                "-p"
                "bedwars"
              ];
              profile = "dev";
            }
          );
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
          # build of it. `packages.smash` wraps this one in the build stamp's
          # environment rather than replacing it: the executable a gate runs and
          # the executable a host runs are the same file, and what differs is
          # three variables. See `stamped` for why the gates are left unwrapped.
          gameBinaries = {
            bedwars = named "bedwars" workspace.binaries.bedwars;
            smash = named "smash" workspace.binaries.smash;
            hyperion-proxy = named "hyperion-proxy" workspace.binaries.hyperion-proxy;
          };

          # The dev-profile game servers the boot gate boots. Same servers as
          # `gameBinaries`, compiled with `debug_assertions` on. Named for the
          # same `meta.mainProgram` reason.
          devGameBinaries = {
            bedwars = named "bedwars" devWorkspace.binaries.bedwars;
            smash = named "smash" devWorkspace.binaries.smash;
          };

          # What build this is, for the strip across a player's screen. Read by
          # `events/smash/src/module/build_stamp.rs`.
          buildStamp =
            let
              # `self.shortRev` exists only on a clean tree and
              # `self.dirtyShortRev` only on a dirty one, and a source with no
              # git in it -- a plain directory, a tarball -- has neither.
              # Dirtiness is carried by its own variable rather than by the
              # `-dirty` suffix nix appends, so the game states the fact instead
              # of parsing a string for it, and so the rev on screen is a hash a
              # person can paste into `git show`.
              rev = self.shortRev or (lib.removeSuffix "-dirty" (self.dirtyShortRev or ""));
            in
            {
              inherit rev;

              # `self.lastModified` is the commit's COMMITTER date, and it is
              # the same number on a dirty tree as on a clean one: nix asks git
              # for the commit either way rather than falling back to a file
              # mtime. Measured on this repo at d55a336 -- 1785386760 clean,
              # 1785386760 dirty, `git log -1 --format=%ct` 1785386760.
              #
              # Committer and not author, which are 1785385579 and 1785386760 on
              # that same commit because it was amended. So a rebased commit's
              # bar reads when the rebase landed rather than when the work was
              # written. That is the right answer for the question this bar
              # exists for -- which build is deployed, and how long ago did that
              # build come into being -- and it is the wrong answer for "when
              # was this change authored", which the bar does not claim.
              #
              # Nothing at all when there is no rev, and that is the point of
              # the conditional rather than a nicety. `lastModified` on a
              # non-git source is the directory's mtime, so without this the bar
              # renders `build unpackaged build · 2026-07-29 20:41 UTC`: a stamp
              # that has just admitted it does not know what it is, timed to the
              # minute. An empty string parses as absent in `BuildStamp::parse`
              # and the bar drops the whole clause.
              time = if rev == "" then "" else toString (self.lastModified or 0);

              dirty = if self ? dirtyShortRev then "1" else "0";
            };

          # The stamp, as an environment a binary is started in.
          #
          # A wrapper and not `env!` in a build script, and the difference is
          # the whole reason this exists: a commit hash compiled into a crate
          # invalidates that crate and everything downstream on every commit, so
          # every push would rebuild the workspace. This derivation is a symlink
          # and three assignments, and it is the only thing a new commit
          # rebuilds.
          #
          # Applied to `packages` and deliberately NOT to the binaries the e2e
          # gates run. cargoUnit content-addresses every crate unit, so a commit
          # that changes no smash source leaves the gate's binary bit for bit
          # identical and the whole gate is reused from the store. Stamping it
          # would move its path on every commit and re-run all fourteen e2e
          # gates for a string none of them reads. The gates cover the *reading*
          # of these variables instead, which is the half that can be wrong in
          # Rust; `checks.build-stamp` below covers the writing.
          #
          # THE COST, AND IT LANDS ON PLAYERS RATHER THAN ON CI. This wrapper's
          # store path moves on every commit, `packages.smash` is it, and
          # `nix/modules/game-server.nix` builds `ExecStart` out of
          # `packages.smash`. So the unit file changes on every commit and
          # `switch-to-configuration` restarts `hyperion-game-server` on every
          # deploy -- including deploys of commits that touch nothing in smash,
          # which used to leave `ExecStart` byte-identical and the running
          # server alone. A restart drops every connected player.
          #
          # The restart is required by the feature: a server cannot report a
          # commit it was not started with. The SCOPE is not. Narrowing it means
          # decoupling the stamp from the unit -- an `EnvironmentFile` the
          # deploy writes, or a stamp read from a path rather than baked into
          # `ExecStart` -- and that is a different change with its own failure
          # mode, namely a stamp that can disagree with the binary beside it.
          # Written down here rather than left to be discovered, because with
          # continuous apply on, "redeployed as main moves" now means every
          # player is dropped as main moves.
          stamped = drv:
            let
              main = drv.meta.mainProgram;
            in
            pkgs.runCommand "${drv.name}-stamped"
              {
                inherit (drv) meta;
                nativeBuildInputs = [ pkgs.makeWrapper ];
                passthru = (drv.passthru or { }) // { unstamped = drv; };
              }
              ''
                makeWrapper ${drv}/bin/${main} "$out/bin/${main}" \
                  --set HYPERION_BUILD_REV ${lib.escapeShellArg buildStamp.rev} \
                  --set HYPERION_BUILD_TIME ${lib.escapeShellArg buildStamp.time} \
                  --set HYPERION_BUILD_DIRTY ${lib.escapeShellArg buildStamp.dirty}
              '';

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

            # The bow read off the wire, as a store-cached gate rather than
            # only the `nix run .#bedwars-bow-e2e` runner. `bow-check.py` fires
            # a real draw and reads the arrow's `ClientboundAddEntity`: its
            # velocity (the charge curve), and since this change its two
            # rotation bytes -- the client-visible heading. An off-axis shot
            # proves the heading is vanilla's projectile convention
            # (`yaw = atan2(dx, dz)`), not the shooter's own look yaw that
            # rendered every arrow mirrored. Same bedwars server and world as
            # the `e2e` gate above.
            bedwars-bow-e2e = e2e.mkCheck {
              name = "hyperion-bedwars-bow-e2e";
              gameServer = gameBinaries.bedwars;
              proxy = gameBinaries.hyperion-proxy;
              client = "bow-check.py";
              clientArgs = [ "--name" "Archer" ];
              needsGenMap = true;
            };

            # The smash bow, the same claims on smash's own projectile path. One
            # client takes the Skeleton kit in the hub and draws Barrage: the
            # arrow must leave the eye (smash fired from the feet), be broadcast
            # every tick so it flies (a smash projectile carries no `Owner`, so
            # `update_projectile_positions` never sent it), and a longer draw
            # must be faster (smash fired every arrow at one fixed speed). A high
            # `SMASH_MIN_PLAYERS` keeps one client in the hub rather than racing
            # a match countdown. smash arenas are `include_str`, so no map.
            smash-bow-e2e = e2e.mkCheck {
              name = "hyperion-smash-bow-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "smash-bow-check.py";
              clientArgs = [ "--name" "Archer" ];
              serverEnv = smashGateLobby.env;
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
              clientArgs = [
                "--full-clients"
                smashGateFullClients
              ];
              serverEnv = smashGateLobby.env;
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

            # The regression gate for the skin and hat bugs, on a real
            # connection: another player's view of a kit wearer carries the
            # committed signed skin and every overlay including the hat.
            smash-skin-e2e = e2e.mkCheck {
              name = "hyperion-smash-skin-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "skin-check.py";
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
              clientArgs = hudAdmins.clientArgs;
              serverEnv = hudAdmins.env;
              timeout = 420;
            };

            # The denial-of-service regression from ENG-10914. One client
            # reaches play and sends the movement packets that used to crash
            # the whole server for everyone -- a finite coordinate past the i16
            # chunk range (x = 2_000_000), then NaN and +inf -- and the gate is
            # green only if the server refuses each and keeps ticking. The
            # client keys its exit code on survival alone, so the event it runs
            # against does not matter. Reverting the move-handler bound turns
            # this red. bedwars is used because it is the pair the proven `e2e`
            # gate already drives.
            oob-move-e2e = e2e.mkCheck {
              name = "hyperion-oob-move-e2e";
              gameServer = gameBinaries.bedwars;
              proxy = gameBinaries.hyperion-proxy;
              client = "client-26.2.py";
              clientArgs = [
                "--name"
                "oob"
                "--oob-move"
              ];
              # The same proven client + server pair the `e2e` gate drives, so
              # reaching play is not in question; bedwars downloads its world at
              # boot, so the sandbox is handed one.
              needsGenMap = true;
              timeout = 300;
            };

            # The daylight cycle, frozen at the protocol level, on a real
            # connection. 26.2 drives the sun from a per-world clock the client
            # advances itself from a `rate` the server sends once; hyperion sent
            # no `SetTime` at all, so a client free-ran its own cycle and the sun
            # drifted. `world-time-check.py` joins, decodes the `SetTime` (id
            # 113) the join path now sends, and asserts the overworld clock
            # arrives with `rate` 0.0 at a fixed day time and never advances over
            # several seconds of ticks. Fail-then-pass by construction: an
            # unpatched server sends no `SetTime`, so the first assertion is red.
            # smash rather than bedwars because the freeze lives in shared join
            # code and smash needs no world downloaded into the sandbox.
            world-time-e2e = e2e.mkCheck {
              name = "hyperion-world-time-e2e";
              gameServer = gameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "world-time-check.py";
              timeout = 180;
            };

            # The dev-profile boot gate. ENG-11000 shipped a singleton that was
            # `world.set` but never registered as a component; a release build
            # compiles the flecs "component is not registered" assert out, so
            # every release gate above -- `world-time-e2e` included -- stayed
            # green while the dev profile the operator actually runs
            # (`nix run .#smash`) aborted on boot and crash-looped the server.
            #
            # These two gates boot each event's game server compiled with
            # `debug_assertions` on (`devGameBinaries`). A singleton that is set
            # but never registered aborts during module init, so the server
            # never opens its port and the driver fails with the panic in the
            # log tail. The client that then joins runs the join path under the
            # same assert, so a registration miss reached only from a join fails
            # here too. The proxy and client stay on the release build: neither
            # carries the bug, and only the game server needs the assert.
            #
            # This is the gate that makes the class impossible rather than
            # catching one instance: any set-but-unregistered singleton in any
            # module either event loads turns this red. A dev build is
            # unoptimised, so both boot and the join get a generous deadline.
            smash-dev-boot-e2e = e2e.mkCheck {
              name = "hyperion-smash-dev-boot-e2e";
              gameServer = devGameBinaries.smash;
              proxy = gameBinaries.hyperion-proxy;
              client = "client-26.2.py";
              clientArgs = [
                "--name"
                "smashdevboot"
              ];
              timeout = 300;
            };

            bedwars-dev-boot-e2e = e2e.mkCheck {
              name = "hyperion-bedwars-dev-boot-e2e";
              gameServer = devGameBinaries.bedwars;
              proxy = gameBinaries.hyperion-proxy;
              client = "client-26.2.py";
              clientArgs = [
                "--name"
                "bedwarsdevboot"
              ];
              needsGenMap = true;
              timeout = 300;
            };

            # `checks.e2e` above took the names the two app wrappers used to
            # hold, and those wrappers still have to pass shellcheck.
            e2e-app = scripts.e2e;
            smash-e2e-app = scripts.smash-e2e;
            smash-bow-e2e-app = scripts.smash-bow-e2e;
            completions-e2e-app = scripts.completions-e2e;
            smash-selector-e2e-app = scripts.smash-selector-e2e;
            smash-identity-e2e-app = scripts.smash-identity-e2e;
            smash-skin-e2e-app = scripts.smash-skin-e2e;
            smash-hotbar-e2e-app = scripts.smash-hotbar-e2e;
            smash-hud-e2e-app = scripts.smash-hud-e2e;
            oob-move-e2e-app = scripts.oob-move-e2e;

            # A gate script may not wait on one packet and assert on the next.
            # The failure that shipped read as a server bug for an afternoon:
            # `smash-selector-e2e` reported an action bar the server had sent
            # 0.2 ms earlier, because the wait was keyed on the chat line in
            # front of it. See nix/verify-wire-assertions.py.
            wire-assertions-are-their-own-wait =
              pkgs.runCommand "hyperion-wire-assertions-are-their-own-wait"
                {
                  nativeBuildInputs = [ pkgs.python3 ];
                }
                ''
                  python3 ${wireAssertionSource}/nix/verify-wire-assertions.py \
                    ${wireAssertionSource} | tee "$out"
                '';

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

            # A signed property is only half of it: the url it covers still has
            # to serve a real skin image and not a broken link. Offline via the
            # pinned, prefetched images in kitSkinImages.
            kit-skins-images =
              pkgs.runCommand "hyperion-kit-skins-images"
                {
                  nativeBuildInputs = [ kitSkinPython ];
                }
                ''
                  python3 ${kitSkinSource}/nix/verify-kit-skin-images.py ${kitSkinImages} | tee "$out"
                '';

            # The pinned world URL still has to be the one the server asks for.
            genmap-url-pinned = e2e.genMapUrlPinned;

            # No two end to end gates may claim one port. See `e2eOffsets`.
            e2e-ports-distinct = e2ePortsDistinct;

            # The deployed smash binary starts in an environment that says what
            # build it is. See `stamped` and
            # `events/smash/src/module/build_stamp.rs`.
            #
            # The Rust half is covered by `cargo test -p smash --test
            # build_stamp`, which pins what each of these three values turns
            # into on screen. Nothing covered that half's *input* until this:
            # a rename, a dropped `--set`, or a `buildStamp` attribute that
            # stopped being threaded all read as a server quietly saying
            # "unpackaged build" to every player, which is exactly the state
            # this whole change exists to end and is invisible from any test
            # that does not look at the wrapper.
            build-stamp =
              pkgs.runCommand "hyperion-build-stamp"
                {
                  wrapper = lib.getExe (stamped gameBinaries.smash);
                  binary = lib.getExe gameBinaries.smash;
                  inherit (buildStamp) rev time dirty;
                }
                ''
                  fail() { echo "FAIL: $1" >&2; exit 1; }

                  # The control. Everything below is a grep, and a grep against
                  # a file that is not there, or is a binary, or is empty,
                  # fails for reasons that have nothing to do with the stamp.
                  [ -f "$wrapper" ] || fail "no wrapper at $wrapper"
                  grep -qF -- "$binary" "$wrapper" \
                    || fail "the wrapper does not exec $binary"

                  for name in REV TIME DIRTY; do
                    grep -q "HYPERION_BUILD_$name=" "$wrapper" \
                      || fail "the wrapper sets no HYPERION_BUILD_$name"
                  done

                  # A rev is empty exactly when nix had no git to ask, which is
                  # true of a tarball and of a plain directory, and is not a
                  # failure.
                  if [ -n "$rev" ]; then
                    # Hex and nothing else. Every other assertion here compares
                    # the wrapper against the same expression that built it, so
                    # on VALUES they are tautologies -- if nix changed the
                    # suffix `removeSuffix` strips, both sides would move
                    # together and this file would stay green while the bar read
                    # `abc1234-dirty + uncommitted changes`. What a short commit
                    # hash looks like is the one property of the value that does
                    # not come from the expression, so it is the only thing here
                    # that can catch a wrong one.
                    case "$rev" in
                      *[!0-9a-f]*)
                        fail "the rev is not a short commit hash: $rev" ;;
                    esac
                    [ -n "$time" ] \
                      || fail "there is a rev but no timestamp beside it"
                    grep -q "HYPERION_BUILD_REV=.*$rev" "$wrapper" \
                      || fail "the wrapper does not carry the rev $rev"
                    grep -q "HYPERION_BUILD_TIME=.*$time" "$wrapper" \
                      || fail "the wrapper does not carry the timestamp $time"
                  else
                    # And no timestamp either. A precise minute next to a stamp
                    # that has just said it does not know what build it is comes
                    # from `lastModified` falling back to a directory mtime, and
                    # is worse than saying nothing.
                    [ -z "$time" ] \
                      || fail "no rev, but a timestamp of $time: that is a directory mtime dressed as a commit"
                    echo "no rev: this flake source is not a git tree" >&2
                  fi
                  grep -q "HYPERION_BUILD_DIRTY=.*'$dirty'" "$wrapper" \
                    || fail "the wrapper does not carry dirty=$dirty"

                  echo "rev=$rev time=$time dirty=$dirty" > "$out"
                '';

            # A colour reaches a client as a component field or not at all.
            smash-text-no-legacy-formatting = textGate;

            # The committed generated sources must match what the pipeline
            # produces, or the copy cargo reads is a fiction.
            minecraft-proto-generated = minecraft.generatedUpToDate;
            minecraft-proto-coverage = minecraft.coverageRatchet;
            tool-paths = minecraft.toolPaths;
            minecraft-literals = minecraft.literalRatchet;
            minecraft-registry-data = minecraft.registryDataUpToDate;
            minecraft-tag-data = minecraft.tagDataUpToDate;
            minecraft-tags-load = minecraft.tagsLoadForClient;
            minecraft-block-states = minecraft.blockStatesUpToDate;
            minecraft-particles = minecraft.particlesUpToDate;
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
                expect "/bin/hyperion-proxy '[::]:25565' --server hyperion-game.ix.internal:35565"
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
              inherit (pkgs) writeShellApplication bash jq;
              names = builtins.attrNames checks;
            };

            # The differential verdict, exercised against fixtures rather than
            # against a real run: no network, no nix, no clock. Every row of the
            # table in nix/ci/delta-gate.sh gets a case, and so does the inverse
            # of every guard, because the failure mode of a test like this is
            # passing for the wrong reason. Six deliberate mutations of the
            # verdict logic were each caught by a named case before this landed.
            delta-gate = pkgs.runCommand "check-delta-gate"
              {
                nativeBuildInputs = [
                  pkgs.bash
                  pkgs.jq
                  pkgs.coreutils
                  pkgs.gnugrep
                  pkgs.gnused
                ];
              }
              ''
                install -m 0755 ${./nix/ci/delta-gate.sh} delta-gate.sh
                install -m 0755 ${./nix/ci/delta-gate-tests.sh} delta-gate-tests.sh
                # The suite locates the library beside itself, so both have to
                # sit in one directory rather than being read from the store.
                bash ./delta-gate-tests.sh
                touch $out
              '';
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
              # `nix run` with no target boots the bedwars stack, the event
              # `packages.default` also builds.
              default = scripts.bedwars;
              # What CI runs. A contributor can run the same one command and
              # see the same verdict, which is what stops the two drifting.
              inherit (checks) flake-gate;
              update-minecraft-data = minecraft.updateScript;
              sync-minecraft-proto = minecraft.syncScript;
              sync-minecraft-literals = minecraft.syncLiteralsScript;
              sync-minecraft-registry-data = minecraft.syncRegistryDataScript;
              sync-minecraft-tag-data = minecraft.syncTagDataScript;
              sync-minecraft-block-states = minecraft.syncBlockStatesScript;
              sync-minecraft-particles = minecraft.syncParticlesScript;
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
            # smash is stamped and the other two are not, because smash is the
            # only one that reads the stamp: `nix/modules/game-server.nix`
            # builds its ExecStart out of this package, so this is what puts a
            # commit on a player's screen on the deployed server.
            smash = stamped gameBinaries.smash;
            inherit (gameBinaries) bedwars hyperion-proxy;
            rust-mc-bot = named "rust-mc-bot" workspace.binaries.rust-mc-bot;

            minecraft-server-jar = minecraft.serverJar;
            minecraft-data = minecraft.generatedData;
            minecraft-decompiled = minecraft.decompiledSources;
            minecraft-physics-sources = minecraft.physicsSources;
            minecraft-client-skin-sources = minecraft.clientSkinSources;
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
            minecraft-particles-rust = minecraft.generatedParticles;
          };

          # What CI enforces of this set is nix/ci/flake-gate.nix.
          inherit checks;

        };

      # The deployed fleet. `nix/fleet/default.nix` says at length why it lives
      # in this repo and why it is not its own flake; the short version is that
      # a second lock is what let the game and its deployment drift apart
      # (ENG-11448), and there is no second lock here.
      #
      # x86_64-linux binaries always, because these are Linux guests whatever
      # machine evaluates them. Taken from `mkSystem` rather than from
      # `self.packages` so the fleet does not depend on the attribute set it
      # contributes to.
      fleet = import ./nix/fleet {
        inherit index;
        guestPackages = (mkSystem "x86_64-linux").packages;
        inherit (self) nixosModules;
      };

      # Force every fleet node's toplevel and record what it resolved to,
      # WITHOUT building any of them. `unsafeDiscardStringContext` is what buys
      # that: the string still has to be computed, so all four module systems
      # evaluate and any option type error, missing attribute or port collision
      # throws here -- but with the context stripped this check does not depend
      # on those closures, so it costs seconds rather than the minutes a real
      # fleet build takes.
      #
      # It is a mitigation, not a restoration. In index this fleet was covered
      # by a REQUIRED context; here nothing is required (ruleset 566717 carries
      # no `required_status_checks`) and every workflow is `workflow_dispatch`.
      # So this catches a broken fleet only when somebody runs it. What the move
      # into this repo does structurally is kill the DRIFT class -- the module
      # and its consumer are now in one commit, so they cannot disagree the way
      # they did in ENG-11448. This check covers the smaller residue: a single
      # commit that breaks both halves at once.
      fleetEvalFor = system: let
        pkgs = nixpkgs.legacyPackages."${system}";
        lines = nixpkgs.lib.mapAttrsToList (
          name: cfg: "${name} ${builtins.unsafeDiscardStringContext cfg.config.system.build.toplevel.drvPath}"
        ) fleet.nixosConfigurations;
      in
      pkgs.runCommand "hyperion-fleet-eval"
        {
          __structuredAttrs = true;
          drvPaths = builtins.concatStringsSep "\n" lines;
          # Guard the guard, BY NAME rather than by count. An empty
          # `nixosConfigurations` would make the line above vacuously true and
          # this check would pass having evaluated nothing -- a green tick
          # meaning "found no nodes", which is indistinguishable from "every
          # node is fine". A bare count would catch that and would also break
          # the day somebody legitimately changes `replicas` in nix/fleet,
          # which is a digit this fleet is meant to be able to turn.
          #
          # Space-joined rather than a list: `__structuredAttrs` renders a Nix
          # list as a bash ARRAY, so `"$nodeNames"` would be its first element
          # only and the loop below would test one name while looking like it
          # tested all of them.
          nodeNames = builtins.concatStringsSep " " (builtins.attrNames fleet.nixosConfigurations);
        }
        ''
          for required in hyperion-game hyperion-proxy-0; do
            case " $nodeNames " in
              *" $required "*) ;;
              *)
                echo "hyperion-fleet-eval: $required is not among the evaluated nodes ($nodeNames)" >&2
                exit 1
                ;;
            esac
          done
          printf '%s\n' "$drvPaths" > "$out"
        '';
    in
    {
      # A NixOS system that imports both modules and nothing else, built by
      # `nix flake check`. Without it the modules are only ever exercised by
      # whoever deploys them, and a typo in an option name is discovered on a
      # host rather than here.
      # The smoke test below plus the deployed fleet's four nodes. Merged into
      # one attribute because the repo already had one: `nixosConfigurations`
      # cannot be defined twice, and the fleet is not a special case that
      # deserves its own namespace.
      nixosConfigurations = fleet.nixosConfigurations // {
        module-smoke-test = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          self.nixosModules.game-server
          self.nixosModules.proxy
          (
            { config, lib, ... }:
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
                gameServer = {
                  host = "hyperion-game.ix.internal";
                  # Read off the game server rather than restated, so this
                  # smoke test cannot pass while the two disagree.
                  port = config.services.hyperion-game-server.port;
                };
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
      # `fleet-eval` joins the enforced set automatically: this repo's gate is
      # subtractive, so every attribute of `checks` is built. See
      # nix/ci/flake-gate.nix.
      checks = forAllSystems (
        system: (mkSystem system).checks // { fleet-eval = fleetEvalFor system; }
      );
      devShells = forAllSystems (system: (mkSystem system).devShells);
      # The fleet's `-system` attrs are exposed under every system, not only
      # x86_64-linux: the machine that types `nix build` contributes a builder,
      # not an identity, and without this the apply command above is a
      # missing-attribute error on the Mac it is most likely to be typed on.
      packages = forAllSystems (system: (mkSystem system).packages // fleet.systemPackages);
    };
}

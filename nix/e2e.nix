# The end to end gates, as one harness.
#
# `nix run .#e2e` and `checks.e2e` are the same script with different binaries
# behind it: the app points it at what cargo just built in the working tree, the
# check points it at what nix built in the store and runs it in the sandbox.
# There is one derivation for the driver so the two cannot drift, which is the
# whole reason this file exists rather than a second copy of the glue.
{
  pkgs,
  lib,
  fileDescriptors,
  sources,
}:
let
  # The scripted clients that verify a skin's Mojang signature the way an
  # online client does need `cryptography`; every other client ignores it. One
  # interpreter for all of them keeps the driver and the checks in step.
  clientPython = pkgs.python3.withPackages (python: [ python.cryptography ]);

  # The world bedwars loads. `hyperion-genmap` downloads this from GitHub the
  # first time a server boots and caches it under the user's cache directory,
  # which a sandbox has no way to do: there is no network in there. So the
  # tarball is pinned by hash here and the cache is seeded with it before the
  # server starts, which turns a download into an input.
  #
  # When repository-owned maps land this whole binding goes away; smash already
  # has them (`events/smash/maps/*.map`, reached by `include_str!`) and so needs
  # nothing seeded.
  genMapUrl = "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";

  genMapArchive = pkgs.fetchurl {
    url = genMapUrl;
    hash = "sha256-ViX+qDEBf++HwbXu1rTAi05Ju/3JAS16Ld4Uq0sStQg=";
  };

  genMap = pkgs.runCommand "hyperion-genmap-world" { } ''
    mkdir -p "$out"
    tar -xzf ${genMapArchive} -C "$out"
    test -d "$out/region" || {
      echo "GenMap.tar.gz no longer unpacks to region/, so the seeded cache" >&2
      echo "would be a directory the server cannot read." >&2
      exit 1
    }
  '';

  # `hyperion_utils::cached_save` keys its cache on the sha256 of the URL text,
  # under the `AppId` in the world when `GenMapModule` is imported. bedwars sets
  # its own `AppId` after that import, so the one that counts is the default
  # `HyperionUtilsModule` writes: github / hyperion-mc / generic.
  genMapKey = builtins.hashString "sha256" genMapUrl;

  # `directories` spells that identity two ways and which one is right depends
  # on the host the check runs on. Both are seeded rather than branching on the
  # platform, because a wrong guess is not silent: with no network in the
  # sandbox a cache miss panics the server on its first boot.
  seedGenMap = ''
    for cache in \
      "$XDG_CACHE_HOME/generic" \
      "$HOME/Library/Caches/github.hyperion-mc.generic"; do
      mkdir -p "$cache"
      ln -s ${genMap} "$cache/${genMapKey}"
    done
  '';

  # The URL above is a copy of one that lives in Rust, and a copy goes stale.
  # If `hyperion-genmap` starts loading a different world the seeded cache
  # quietly stops being the world under test, so this fails the build instead.
  # It is also the first thing to delete once world loading stops downloading.
  genMapUrlPinned = pkgs.runCommand "hyperion-genmap-url-pinned" { } ''
    if ! grep -qF '${genMapUrl}' ${sources.genmap}; then
      echo "nix/e2e.nix pins" >&2
      echo "  ${genMapUrl}" >&2
      echo "but crates/hyperion-genmap/src/lib.rs no longer names it, so the" >&2
      echo "sandboxed checks would seed a cache the server never reads and" >&2
      echo "then fail with no network to fall back on. Repin the URL and its" >&2
      echo "hash, or delete both if the world now comes from the repository." >&2
      exit 1
    fi
    touch "$out"
  '';

  # mTLS between the game server and the proxy, minted in a derivation rather
  # than by `nix run .#certs` reaching into the working tree. Throwaway by
  # construction: nothing outside these checks trusts the CA, and the CA key is
  # deleted rather than kept.
  certs = pkgs.runCommand "hyperion-e2e-certs" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir -p "$out"
    cd "$out"

    openssl req -new -nodes -newkey rsa:2048 -keyout root_ca.pem \
      -x509 -out root_ca.crt -days 36500 -subj '/CN=hyperion-e2e-ca'

    for who in server proxy; do
      openssl req -nodes -newkey rsa:2048 \
        -keyout "''${who}_private_key.pem" -out "$who.csr" \
        -subj "/CN=hyperion-e2e-$who"
      # The SAN must cover the address the peer dials or the handshake fails
      # with "certificate not valid for name".
      openssl x509 -req -in "$who.csr" -CA root_ca.crt -CAkey root_ca.pem \
        -CAcreateserial -out "$who.crt" -days 36500 -sha256 \
        -extfile <(printf 'subjectAltName=DNS:localhost,IP:127.0.0.1')
      rm -f "$who.csr"
    done
    rm -f root_ca.srl root_ca.pem
  '';

  # The scripted clients read four things off disk: each other, the registry
  # contents they check the server's tags against, `protocol.json` -- which is
  # how they turn a registry id on the wire into a name -- and the committed
  # kit skins a profile has to match. All four are under one root because the
  # clients reach for them by repository-relative path.
  #
  # `protocol.json` and not the generated Rust: two clients used to scrape the
  # tables out of `src/generated/registry.rs` with a regex, and both broke the
  # day that file became a directory. A path into somebody else's source is not
  # an interface.
  clients = lib.fileset.toSource {
    root = sources.root;
    fileset = lib.fileset.unions [
      (lib.fileset.fileFilter (file: file.hasExt "py") sources.tools)
      sources.protoSource
      sources.protocolJson
      sources.kitSkins
    ];
  };

  # One script, both callers. Everything that varies is an environment
  # variable, and the flags the two binaries take are written down exactly once
  # -- the pair most likely to drift, because a renamed flag breaks a gate
  # nobody runs until CI does.
  driver = pkgs.writeShellApplication {
    name = "hyperion-e2e-driver";
    runtimeInputs = [ clientPython ];
    text = ''
      # Job control, so each background process is its own group. `cargo run`
      # forks the binary under test, and killing only cargo would leave the
      # server holding its port for the next run.
      set -m

      : "''${HYPERION_E2E_GAME_SERVER:?the command that starts the game server}"
      : "''${HYPERION_E2E_PROXY:?the command that starts the proxy}"
      : "''${HYPERION_E2E_CLIENT:?the client script and its arguments}"
      : "''${HYPERION_E2E_CERTS:?a directory holding root_ca.crt and the two leaf pairs}"

      # Unset means "find a free set". A check has no reason to care which
      # ports it uses and every reason not to collide: on Linux the sandbox
      # has its own network namespace, but on darwin there is no such thing
      # and a build shares the host's loopback with everything else on the
      # machine. A fixed 47565 lost that race to an unrelated server the first
      # time this ran. Every socket is held open until all the numbers are
      # read, so no two can come back equal.
      #
      # Three and not two because a gate may ask for an operator console, which
      # listens on a port of its own. Picked here rather than derived from one
      # of the others -- `server_port + 1` is a free number right up until the
      # run that finds it taken -- and picked whether or not this run wants
      # one, so the held-open guarantee covers the same set every time.
      picked_player=""
      picked_server=""
      picked_console=""
      if [ -z "''${HYPERION_PLAYER_PORT:-}" ] || [ -z "''${HYPERION_SERVER_PORT:-}" ] \
        || [ -z "''${HYPERION_CONSOLE_PORT:-}" ]; then
        read -r picked_player picked_server picked_console < <(python3 -c "
      import socket
      held = [socket.socket() for _ in range(3)]
      for sock in held:
          sock.bind(('127.0.0.1', 0))
      print(' '.join(str(sock.getsockname()[1]) for sock in held))
      ")
      fi
      player_port="''${HYPERION_PLAYER_PORT:-$picked_player}"
      server_port="''${HYPERION_SERVER_PORT:-$picked_server}"
      console_port="''${HYPERION_CONSOLE_PORT:-$picked_console}"
      bind="''${HYPERION_E2E_BIND:-127.0.0.1}"
      certs="$HYPERION_E2E_CERTS"
      log="''${HYPERION_E2E_LOG:-$(mktemp -t hyperion-e2e.XXXXXX)}"
      # A cold `nix run` compiles two binaries behind this; a check has them
      # already and is ready in seconds. A deadline rather than a sleep, so the
      # fast case does not pay for the slow one.
      ready_timeout="''${HYPERION_E2E_TIMEOUT:-900}"

      read -ra game_server <<< "$HYPERION_E2E_GAME_SERVER"
      read -ra proxy <<< "$HYPERION_E2E_PROXY"
      read -ra client <<< "$HYPERION_E2E_CLIENT"

      # An operator console, for the one gate whose question is about it. Off
      # otherwise: a console is an admin port, and opening one on every gate
      # would be a surface nothing else here needs.
      #
      # The server and the client are handed the same two facts from one place,
      # because an address written down twice is the pair most likely to drift.
      # Arrays rather than strings, so the absent case expands to no arguments
      # at all rather than to one empty one.
      game_server_console=()
      client_console=()
      if [ -n "''${HYPERION_E2E_CONSOLE:-}" ]; then
        token_file="''${HYPERION_E2E_TOKEN_FILE:-$(mktemp -t hyperion-console-token.XXXXXX)}"
        # Deliberately a token carrying `+` and `/`. Standard base64 emits
        # both, an operator who generates one with `base64` gets them, and `+`
        # in a query used to come back 401 because the decoder read it as a
        # space. A token without them leaves that fix untested.
        python3 -c "
      import base64, os, sys
      sys.stdout.write('+/' + base64.b64encode(os.urandom(18)).decode())
      " > "$token_file"
        game_server_console=(--console-bind "$bind:$console_port" --console-token-file "$token_file")
        client_console=(--console "$bind:$console_port" --token-file "$token_file")
      fi

      echo "stack log: $log"

      "''${game_server[@]}" \
        --ip "$bind" --port "$server_port" \
        --root-ca-cert "$certs/root_ca.crt" \
        --cert "$certs/server.crt" \
        --private-key "$certs/server_private_key.pem" \
        ''${game_server_console[@]+"''${game_server_console[@]}"} \
        < /dev/null >> "$log" 2>&1 &
      game_pid=$!

      # shellcheck disable=SC2329  # called below and from the EXIT trap
      cleanup() {
        for pid in "''${proxy_pid:-}" "$game_pid"; do
          [ -n "$pid" ] || continue
          kill -- "-$pid" >> "$log" 2>&1 || kill "$pid" >> "$log" 2>&1 || true
          wait "$pid" >> "$log" 2>&1 || true
        done
      }
      trap cleanup EXIT
      # `timeout` around this script sends TERM, and bash runs an EXIT trap on
      # a signal only when that signal is trapped too. Without this a check
      # that hits its cap leaves the stack running.
      #
      # WITH this, it still does, and do not read the paragraph above as saying
      # otherwise (ENG-11370). A trap is dispatched between commands, never
      # during one, and this script spends its whole run inside the foreground
      # `python3 ... | tee` pipeline below. The TERM is queued there until the
      # client finishes on its own -- which is the thing the cap exists to
      # bound. Measured: `smash-e2e` declares `timeout = 480`, its derivation
      # carries `timeout 480 hyperion-e2e-driver`, and the run took 633s with
      # the client still logging at 631.23s. `timeout` did fire (nix reports
      # exit 124) and stopped nothing. So today the cap only relabels the exit
      # code of a run that completed, which also makes "timed out" and "failed
      # its assertions" indistinguishable from outside.
      #
      # The fix is to background the client and `wait` on it, because bash does
      # interrupt `wait` to dispatch a trap; ENG-11370 carries the patch and the
      # two directions it has to be watched failing in first. Left undone here
      # deliberately: this is shared by every e2e gate and getting it wrong
      # turns all of them green.
      trap 'exit 143' TERM INT

      fail() {
        echo "$1" >&2
        echo "tail of $log:" >&2
        tail -60 "$log" >&2
        exit 1
      }

      # Waits for a port to answer, or fails the run saying which one did not.
      # Each call gets its own deadline. Probing the game server's port costs a
      # line of "tls handshake eof" in its log per attempt, because a probe
      # connects and hangs up; that noise is the price of not guessing.
      await_port() {
        local port="$1" who="$2" pid="$3"
        local deadline=$(( SECONDS + ready_timeout ))
        until python3 -c "
      import socket, sys
      s = socket.socket()
      s.settimeout(1)
      sys.exit(0 if s.connect_ex(('127.0.0.1', $port)) == 0 else 1)
      "; do
          if [ "$SECONDS" -ge "$deadline" ]; then
            fail "the $who never opened 127.0.0.1:$port"
          fi
          if ! kill -0 "$pid" >> "$log" 2>&1; then
            fail "the $who exited before opening 127.0.0.1:$port"
          fi
          sleep 2
        done
      }

      # In this order, and waiting in between, because the proxy binds its
      # listener immediately and only then dials the game server behind it.
      # Started together, the player port is open for as long as the game
      # server takes to start -- 95 seconds of it on a cold `nix run`, where
      # cargo is still compiling -- and a client that connects in that window
      # dies on a read timeout that reads exactly like a protocol bug. That
      # cost an agent a full cycle to diagnose once already (ENG-10450), and
      # then cost this one another when the two were started together. The
      # price is that a cold `nix run` compiles the two binaries in sequence
      # rather than at once; a check has both already built.
      await_port "$server_port" "game server" "$game_pid"

      # The console binds inside `init_game`, so a client that reaches for it
      # before the server gets there sees connection refused and reports it as
      # the console being broken. Waiting names the right thing instead.
      if [ -n "''${HYPERION_E2E_CONSOLE:-}" ]; then
        await_port "$console_port" "console" "$game_pid"
      fi

      # Best effort: a sandbox can hold a hard limit below this, and these
      # gates drive four clients rather than the few thousand bots it is for.
      ulimit -Sn ${fileDescriptors} || true

      "''${proxy[@]}" \
        --server "127.0.0.1:$server_port" \
        --root-ca-cert "$certs/root_ca.crt" \
        --cert "$certs/proxy.crt" \
        --private-key "$certs/proxy_private_key.pem" \
        "$bind:$player_port" \
        < /dev/null >> "$log" 2>&1 &
      proxy_pid=$!

      await_port "$player_port" "proxy" "$proxy_pid"

      echo "stack up on 127.0.0.1:$player_port"

      # Not `exec`: replacing this shell would skip the EXIT trap and orphan
      # both processes, and the next run would die on "address already in use".
      rc=0
      client_log="$(mktemp -t hyperion-e2e-client.XXXXXX)"
      python3 "''${client[@]}" --host 127.0.0.1 --port "$player_port" \
        ''${client_console[@]+"''${client_console[@]}"} "$@" \
        2>&1 | tee "$client_log" || rc=$?

      # A client that finished its checks proves nothing if the server died
      # while it was reading. It has: the movement handler aborted the process
      # on the first step a player took (hyperion#987), and the client saw only
      # its own read timeout, which it treats as a clean end of session. So ask
      # the game server directly whether it is still listening.
      if ! python3 -c "
      import socket, sys
      s = socket.socket()
      s.settimeout(2)
      sys.exit(0 if s.connect_ex(('127.0.0.1', $server_port)) == 0 else 1)
      "; then
        fail "the game server stopped listening during the session"
      fi

      if [ "$rc" -ne 0 ]; then
        echo "tail of $log:" >&2
        tail -40 "$log" >&2
        # The client says why it failed in the middle of its transcript, well
        # before the packet census it ends with, and a build failure is read
        # through a fixed tail. Repeating the verdict last is what puts the
        # reason in the excerpt CI prints rather than a page of packet counts.
        echo "" >&2
        echo "the client's verdict was failure. What it reported:" >&2
        grep -E 'RESULT:|Traceback|Error' "$client_log" >&2 || true
      fi
      exit "$rc"
    '';
  };

  # A sandboxed gate: nix builds the binaries, the sandbox boots them on
  # loopback, the scripted client drives them, and the client's verdict is the
  # build result.
  #
  # Ports are left unset so the driver picks a free pair. On Linux the sandbox
  # has its own network namespace and any port would do; on darwin
  # `__darwinAllowLocalNetworking` puts the build on the host's loopback, where
  # a fixed number is a race against every other process on the machine.
  mkCheck =
    {
      name,
      gameServer,
      proxy,
      client,
      clientArgs ? [ ],
      # Extra arguments for the game server, after the five the driver always
      # passes. For a gate whose question is about a server configured a
      # particular way; `serverEnv` cannot answer it, because
      # `hyperion_event_runner` reads the environment all-or-nothing and falls
      # back to the command line the moment one variable is missing.
      gameServerArgs ? [ ],
      # Environment for the game server process. A gate whose question needs a
      # server configured differently from the one the product ships says so
      # here, rather than the client inferring it.
      serverEnv ? { },
      # Start the game server with an operator console and tell the client
      # where to find it. A boolean and not a number: the driver picks the port
      # beside the other two, and a gate naming its own would be the fixed-port
      # race this file already learned about once.
      console ? false,
      needsGenMap ? false,
      timeout ? 300,
    }:
    pkgs.runCommand name
      {
        nativeBuildInputs = [
          clientPython
          driver
        ];
        # The game server builds a reqwest client at startup, before it knows
        # whether it will ever make a request, and reqwest refuses to construct
        # one without a trust store. A darwin build finds the keychain and a
        # NixOS host often leaks /etc/ssl into the sandbox, so this check only
        # looked green on the machines it was written on: on a stock Linux
        # store it dies with `No CA certificates were loaded from the system`
        # before the server opens its port. Measured on ubuntu-latest, GitHub
        # Actions run 30341722090, where all three e2e gates failed this way
        # and nothing else. Naming the bundle turns a leak into an input.
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        # Without this a darwin build gets no loopback at all and the proxy
        # cannot reach the game server. Linux ignores it and uses the sandbox's
        # own network namespace, which already has loopback up.
        __darwinAllowLocalNetworking = true;
      }
      ''
        export HOME="$NIX_BUILD_TOP/home"
        export XDG_CACHE_HOME="$HOME/.cache"
        mkdir -p "$XDG_CACHE_HOME"
        ${lib.optionalString needsGenMap seedGenMap}

        ${lib.concatStringsSep "\n" (
          lib.mapAttrsToList (
            name: value: "export ${name}=${lib.escapeShellArg (toString value)}"
          ) serverEnv
        )}
        export HYPERION_E2E_GAME_SERVER="${lib.getExe gameServer} ${lib.escapeShellArgs gameServerArgs}"
        export HYPERION_E2E_PROXY="${lib.getExe proxy}"
        export HYPERION_E2E_CLIENT="${clients}/tools/${client} ${lib.escapeShellArgs clientArgs}"
        export HYPERION_E2E_CERTS="${certs}"
        export HYPERION_E2E_LOG="$NIX_BUILD_TOP/stack.log"
        # The binaries are already built, so anything past this is a hang
        # rather than a slow compile.
        export HYPERION_E2E_TIMEOUT=120
        ${lib.optionalString console ''
          export HYPERION_E2E_CONSOLE=1
          # Named rather than left to `mktemp`, so a run that fails leaves
          # the token beside the rest of the build's evidence.
          export HYPERION_E2E_TOKEN_FILE="$NIX_BUILD_TOP/console-token"
        ''}

        timeout ${toString timeout} hyperion-e2e-driver
        touch "$out"
      '';
in
{
  inherit
    driver
    certs
    clients
    genMap
    genMapUrlPinned
    mkCheck
    ;
}

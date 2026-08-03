# The hyperion game server: one process holding the world, which proxies dial
# into. It never faces the internet.
#
# Traffic direction is what makes the topology simple. Proxies connect to the
# game server, not the other way round, so this node needs one address its
# proxies can reach and nothing else. Put it on a private network and give the
# proxies the public ones.
#
# ---------------------------------------------------------------------------
# RELOAD, AND THE ONE RULE THAT MAKES IT WORK
# ---------------------------------------------------------------------------
#
# `switch-to-configuration` reloads a unit whose `[Service]` section is
# byte-identical and whose `X-Reload-Triggers` differs, and restarts it
# otherwise. So the rules dylib's store path may appear in exactly one place in
# this file -- `reloadTriggers` -- and `ExecStart` must reach the same file
# through a path that does not move, which is what `/etc/hyperion` is for.
#
# That is a property worth checking rather than trusting, because getting it
# wrong is invisible: every gate passes, the deploy succeeds, and the only
# symptom is that players were dropped on a deploy that should have been
# invisible to them. `checks.hot-reload-unit-split` renders this unit for two
# builds of the rules and asserts that `[Service]` did not move.
{ hyperionPackages }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.hyperion-game-server;
  common = import ./common.nix { inherit lib; };
  inherit (lib) mkOption mkEnableOption mkIf types escapeShellArgs optionals;

  # Where the deploy writes what the server reads, and where the server reads it
  # from. One directory, named after the engine rather than after a game,
  # because a host runs one game server and the build stamp in it is the host's.
  etcDir = "hyperion";
  rulesFile = "/etc/${etcDir}/${cfg.event}-rules.so";
  stampDir = "/etc/${etcDir}";

  # `RuntimeDirectory` below is what creates this, and `ProtectSystem = strict`
  # is what makes it the only writable place the unit has.
  runtimeDir = "hyperion-game-server";
  socket = "/run/${runtimeDir}/reload.sock";

  reloadable = cfg.rules != null;
in
{
  options.services.hyperion-game-server = {
    enable = mkEnableOption "the hyperion game server";

    package = mkOption {
      type = types.package;
      default = hyperionPackages.${pkgs.stdenv.hostPlatform.system}.bedwars;
      defaultText = "the bedwars binary for this system";
      description = ''
        The event to run. One binary is one game: `bedwars` and `smash` are
        separate packages rather than a flag, because an event is compiled in.
      '';
    };

    event = mkOption {
      type = types.str;
      default = "hyperion";
      description = ''
        What this deployment calls the game. Used only to name the rules file
        in `/etc/hyperion`, so that somebody reading that directory on a host
        can tell which game's rules are sitting in it.
      '';
      example = "smash";
    };

    rules = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = ''
        The reloadable rules dylib, as a full path to the `.so` inside its
        store path. `null` runs the server with no reloadable rules at all, and
        every deploy is then a restart.

        This store path is put in `X-Reload-Triggers` and **nowhere else in the
        unit**. That is the whole mechanism: a deploy that changes only this
        leaves `[Service]` untouched, and a unit whose `[Service]` did not move
        is one systemd reloads instead of restarting.
      '';
      example = lib.literalExpression ''"''${hyperionPackages.x86_64-linux.smash-rules}/lib/libsmash_rules.so"'';
    };

    reloadClient = mkOption {
      type = types.package;
      default = hyperionPackages.${pkgs.stdenv.hostPlatform.system}.hyperion-dylibs;
      defaultText = "the engine dylib set for this system, which ships the client";
      description = ''
        The package providing `hyperion-reload-client`, which is what
        `ExecReload` runs. It ships with the engine rather than with an event
        because `ExecReload` is part of `[Service]`: a path that moved when an
        event's rules moved would restart the server on exactly the deploys
        this whole feature exists to make invisible.
      '';
    };

    buildStamp = mkOption {
      description = ''
        What build this deployment is, written into `/etc/hyperion` for the
        server to read at runtime.

        Files rather than environment variables, and that is the point: a
        process's environment is fixed at `exec`, so a server that reloaded its
        rules without restarting would report the build it started as forever.
        See `events/smash/src/module/build_stamp.rs`.
      '';
      default = { };
      type = types.submodule {
        options = {
          rev = mkOption {
            type = types.str;
            default = "";
            description = "The short commit hash, or empty when nothing knows.";
          };
          committedAt = mkOption {
            type = types.nullOr types.int;
            default = null;
            description = ''
              When that commit was made, in whole seconds since the unix epoch.
              `null` when there is no commit to date, which is the only honest
              answer for a source with no git in it -- a directory's mtime is
              not a commit date, and a bar reading `unpackaged build · 2h ago`
              would be a stamp that had just admitted it does not know what it
              is, timed to the minute.
            '';
          };
          dirty = mkOption {
            type = types.bool;
            default = false;
            description = ''
              The working tree had uncommitted changes, so `rev` names a tree
              this build was not made from. The game draws the bar in red.
            '';
          };
        };
      };
    };

    address = mkOption {
      type = types.str;
      default = "::";
      description = ''
        What to bind. The default takes every address, which is safe only
        because this port is not routed from the internet. Narrow it to the
        private interface if that assumption does not hold in your deployment.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 35565;
      description = "The port proxies dial.";
    };

    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Arguments appended verbatim after the ones this module builds.";
    };

    pki = common.pkiOptions;
  };

  config = mkIf cfg.enable {
    # Read by the server at startup and on every reload. Nothing here reaches
    # the unit file, which is why a new build of the rules can land without
    # `[Service]` changing.
    environment.etc = {
      "${etcDir}/build-rev".text = cfg.buildStamp.rev;
      "${etcDir}/build-time".text =
        if cfg.buildStamp.committedAt == null then "" else toString cfg.buildStamp.committedAt;
      "${etcDir}/build-dirty".text = if cfg.buildStamp.dirty then "1" else "0";
    } // lib.optionalAttrs reloadable {
      "${etcDir}/${cfg.event}-rules.so".source = cfg.rules;
    };

    systemd.services.hyperion-game-server = {
      description = "hyperion game server";
      # network-online rather than network: the bind fails outright if the
      # address is not up yet, and a restart loop is a worse diagnostic than
      # waiting.
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      # THE ONLY PLACE THE RULES STORE PATH MAY APPEAR. See the header.
      #
      # The build stamp is in here too, and for a reason that is not obvious: a
      # commit that changes nothing the server links -- documentation, CI, the
      # proxy -- still changes what `/etc/hyperion/build-rev` says, and without
      # a reload the running server would go on telling players about the commit
      # before it. That is the exact question the bar exists to answer, so it
      # has to stay true. Such a reload re-opens a byte-identical dylib and
      # rewrites nothing; it is the code-only case `docs/hot-reload.md` measured
      # at a few milliseconds, and it cannot be refused, because a schema can
      # only move when the dylib does.
      reloadTriggers = optionals reloadable [
        cfg.rules
        cfg.buildStamp.rev
        (toString cfg.buildStamp.committedAt)
        (if cfg.buildStamp.dirty then "dirty" else "clean")
      ];

      serviceConfig = common.hardening // {
        Type = "simple";
        ExecStart = escapeShellArgs ([
          "${cfg.package}/bin/${cfg.package.meta.mainProgram}"
          "--ip" cfg.address
          "--port" (toString cfg.port)
          "--root-ca-cert" (toString cfg.pki.rootCaCert)
          "--cert" (toString cfg.pki.cert)
          "--private-key" (toString cfg.pki.privateKey)
        ] ++ optionals reloadable [
          # Stable paths, every one of them. Nothing on this line moves when a
          # new build of the rules is deployed.
          "--rules" rulesFile
          "--reload-socket" socket
          "--build-stamp" stampDir
        ] ++ cfg.extraArgs);

        Restart = "on-failure";
        RestartSec = "2s";
        StateDirectory = "hyperion-game-server";

        # The world is memory mapped and a full server holds thousands of
        # connections' worth of buffers, so the default 1024 descriptors runs
        # out long before anything else does.
        LimitNOFILE = 1048576;
      } // lib.optionalAttrs reloadable {
        # The request carries no arguments -- the module path and the stamp
        # directory were fixed at startup -- so this is a constant string, and a
        # constant string is one that cannot move a deploy from reload back to
        # restart. The client exits non-zero on a refusal, which fails the
        # `systemctl reload` that asked rather than leaving the reason in a log
        # nobody reads.
        ExecReload = escapeShellArgs [
          (lib.getExe' cfg.reloadClient "hyperion-reload-client")
          socket
        ];

        # Where the reload socket lives. `ProtectSystem = strict` leaves the
        # unit nothing else it may write to.
        RuntimeDirectory = runtimeDir;

        # AF_UNIX for that socket. `common.hardening` grants the two IP families
        # and nothing else, so without this the server dies at startup on
        # `socket(AF_UNIX): Address family not supported by protocol` -- and
        # only on a real host, because nothing in a test or a gate runs under
        # this filter.
        RestrictAddressFamilies = common.hardening.RestrictAddressFamilies ++ [ "AF_UNIX" ];
      };
    };
  };
}

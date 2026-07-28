# The hyperion game server: one process holding the world, which proxies dial
# into. It never faces the internet.
#
# Traffic direction is what makes the topology simple. Proxies connect to the
# game server, not the other way round, so this node needs one address its
# proxies can reach and nothing else. Put it on a private network and give the
# proxies the public ones.
{ hyperionPackages }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.hyperion-game-server;
  common = import ./common.nix { inherit lib; };
  inherit (lib) mkOption mkEnableOption mkIf types escapeShellArgs;
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
    systemd.services.hyperion-game-server = {
      description = "hyperion game server";
      # network-online rather than network: the bind fails outright if the
      # address is not up yet, and a restart loop is a worse diagnostic than
      # waiting.
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = common.hardening // {
        Type = "simple";
        ExecStart = escapeShellArgs ([
          "${cfg.package}/bin/${cfg.package.meta.mainProgram}"
          "--port" (toString cfg.port)
          "--ip" cfg.address
          "--root-ca-cert" (toString cfg.pki.rootCaCert)
          "--cert" (toString cfg.pki.cert)
          "--private-key" (toString cfg.pki.privateKey)
        ] ++ cfg.extraArgs);
        Restart = "on-failure";
        RestartSec = "2s";
        StateDirectory = "hyperion-game-server";

        # The world is memory mapped and a full server holds thousands of
        # connections' worth of buffers, so the default 1024 descriptors runs
        # out long before anything else does.
        LimitNOFILE = 1048576;
      };
    };
  };
}

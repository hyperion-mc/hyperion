# The hyperion proxy: the process players connect to. It terminates the
# Minecraft connection, forwards bytes to the game server without reading them,
# and is the only part of the system that faces the internet.
#
# Several proxies share one game server. Each holds its own player connections
# and multiplexes them onto a single link, so adding one is adding a node here
# rather than changing anything on the game server.
{ hyperionPackages }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.hyperion-proxy;
  common = import ./common.nix { inherit lib; };
  inherit (lib) mkOption mkEnableOption mkIf types escapeShellArgs;
in
{
  options.services.hyperion-proxy = {
    enable = mkEnableOption "the hyperion proxy";

    package = mkOption {
      type = types.package;
      default = hyperionPackages.${pkgs.stdenv.hostPlatform.system}.hyperion-proxy;
      defaultText = "the hyperion-proxy binary for this system";
      description = "The proxy binary.";
    };

    listen = mkOption {
      type = types.str;
      default = "[::]:25565";
      description = ''
        Where players connect. 25565 is the port a Minecraft client tries when
        the address carries no port, so moving it means every player has to
        type one.
      '';
    };

    # Two typed fields rather than one `host:port` string. The string form
    # made the port a second copy of `services.hyperion-game-server.port`,
    # free to drift from it, and nothing rendered the pair in one place.
    gameServer = {
      host = mkOption {
        type = types.str;
        example = "hyperion-game.ix.internal";
        description = ''
          The game server's name.

          This must be a name, not an address: it becomes the TLS server name
          the proxy expects on the game server's certificate, so an address
          here fails the handshake against a certificate issued for a name,
          and the failure reads as a connection problem rather than a naming
          one.

          It must also be a name the guest can resolve. On ix that is the
          `ix.internal` zone; a bare `.internal` is not a zone ix serves, so
          it is forwarded upstream and returns NXDOMAIN.
        '';
      };

      port = mkOption {
        type = types.port;
        default = 35565;
        description = ''
          The port the game server listens on.

          Where both services evaluate together, set this from
          `config.services.hyperion-game-server.port` rather than repeating
          the number, so the two cannot disagree.
        '';
      };
    };

    openFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Open the listen port. A proxy nobody can reach is not one.";
    };

    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Arguments appended verbatim after the ones this module builds.";
    };

    pki = common.pkiOptions;
  };

  config = mkIf cfg.enable {
    systemd.services.hyperion-proxy = {
      description = "hyperion proxy";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = common.hardening // {
        Type = "simple";
        ExecStart = escapeShellArgs ([
          "${cfg.package}/bin/${cfg.package.meta.mainProgram}"
          cfg.listen
          "--server" "${cfg.gameServer.host}:${toString cfg.gameServer.port}"
          "--root-ca-cert" (toString cfg.pki.rootCaCert)
          "--cert" (toString cfg.pki.cert)
          "--private-key" (toString cfg.pki.privateKey)
        ] ++ cfg.extraArgs);
        # The proxy retries the game server itself, so a restart here means the
        # proxy died rather than that the server is down.
        Restart = "on-failure";
        RestartSec = "2s";
        StateDirectory = "hyperion-proxy";

        # One descriptor per player, plus the link to the game server. The
        # default 1024 caps the server at a few hundred players, which is two
        # orders of magnitude below what this is for.
        LimitNOFILE = 1048576;
      };
    };

    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [
      (lib.toInt (lib.last (lib.splitString ":" cfg.listen)))
    ];
  };
}

# Shared option shapes for the two hyperion services.
#
# The game server and the proxy authenticate to each other with mutual TLS,
# and both refuse to start without all three files, so the paths are options
# with no defaults rather than guesses. Where the files come from is the
# deployment's problem: a secret manager, a provisioning unit, or for local
# experiments `nix run .#certs`.
{ lib }:
let
  inherit (lib) mkOption types;
in
{
  pkiOptions = {
    rootCaCert = mkOption {
      type = types.path;
      description = ''
        The certificate authority both sides trust. A peer presenting a
        certificate this does not sign is refused, which is the whole point:
        the game server accepts commands from its proxies and from nothing
        else.
      '';
      example = "/var/lib/hyperion-pki/root_ca.crt";
    };

    cert = mkOption {
      type = types.path;
      description = "This node's certificate, signed by `rootCaCert`.";
      example = "/var/lib/hyperion-pki/node.crt";
    };

    privateKey = mkOption {
      type = types.path;
      description = ''
        The private key for `cert`. Readable only by this service's user, so
        it belongs in a secret store or a `systemd` credential rather than in
        the nix store, which is world readable.
      '';
      example = "/var/lib/hyperion-pki/node_private_key.pem";
    };
  };

  # Everything a long-lived network service can give up and still run. Written
  # once here because the two units want the same answer and a copy would drift.
  hardening = {
    DynamicUser = true;
    NoNewPrivileges = true;
    PrivateDevices = true;
    PrivateTmp = true;
    ProtectClock = true;
    ProtectControlGroups = true;
    ProtectHome = true;
    ProtectHostname = true;
    ProtectKernelLogs = true;
    ProtectKernelModules = true;
    ProtectKernelTunables = true;
    ProtectProc = "invisible";
    ProtectSystem = "strict";
    RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
  };
}

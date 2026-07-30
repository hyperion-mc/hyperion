# Mutual TLS between the proxies and the game server, minted on each node from
# one shared certificate authority.
#
# Both hyperion binaries refuse to start without a CA certificate, a leaf
# certificate and its key, and the game server accepts commands only from
# peers the CA signed. That is what keeps a client that finds the game server's
# address from talking to it directly.
#
# The distribution problem has one shared secret rather than three: every node
# gets the same CA key and mints its own leaf at activation. Nothing has to
# copy a certificate from one VM to another, and a private key never enters the
# nix store, which is world readable.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.hyperion.pki;
  dir = "/var/lib/hyperion-pki";
in {
  options.hyperion.pki = {
    caKeyFile = lib.mkOption {
      type = lib.types.path;
      default = ./dev-ca.key;
      defaultText = "./dev-ca.key, the throwaway authority checked into this example";
      description = ''
        The certificate authority's private key, shared by every node.

        The default is a key committed next to this file so the example runs
        with no secret plumbing. It is public, so anything using it is a
        demonstration and not a deployment. For anything real, deliver a key
        out of band and point this at it.
      '';
    };

    serverName = lib.mkOption {
      type = lib.types.str;
      description = ''
        The name the game server's certificate is issued for.

        The proxy's `--server` host becomes the TLS server name it expects, so
        this and that must be the same string. An address on either side fails
        the handshake, and the failure reads as a connection problem rather
        than a naming one.
      '';
    };
  };

  config = {
    # Who may read the minted material. Both hyperion services run with
    # `DynamicUser`, so they have no stable uid to own these files with and no
    # way to read root-only ones; a supplementary group is the one thing
    # systemd will grant a dynamic user.
    users.groups.hyperion-pki = {};

    systemd.services = lib.mkMerge [
      (lib.mkIf config.services.hyperion-game-server.enable {
        hyperion-game-server.serviceConfig.SupplementaryGroups = ["hyperion-pki"];
      })
      (lib.mkIf config.services.hyperion-proxy.enable {
        hyperion-proxy.serviceConfig.SupplementaryGroups = ["hyperion-pki"];
      })
      {
        # Ordered before both services rather than merely enabled: a service that
        # started first would read a directory that does not exist yet and die on
        # a missing file, which reads as a configuration error.
        hyperion-pki = {
          description = "mint this node's hyperion TLS certificate";
          wantedBy = ["multi-user.target"];
          before = ["hyperion-game-server.service" "hyperion-proxy.service"];

          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            StateDirectory = "hyperion-pki";
            StateDirectoryMode = "0750";
            # Root writes, the group reads. systemd applies both to the state
            # directory it creates, so the readers can traverse it.
            Group = "hyperion-pki";
            UMask = "0027";
          };

          path = [pkgs.openssl];

          script = ''
            set -euo pipefail
            cd ${dir}

            install -m 0400 ${cfg.caKeyFile} ca.key

            # The CA certificate is derived from the key rather than shipped
            # beside it, so the two cannot drift apart. Every node computes the
            # same one because they share the key and these fields.
            openssl req -x509 -new -key ca.key -sha256 -days 3650 \
              -subj '/CN=hyperion-fleet-ca' \
              -addext 'basicConstraints=critical,CA:TRUE' \
              -addext 'keyUsage=critical,keyCertSign' \
              -out root_ca.crt

            openssl genpkey -algorithm ED25519 -out node_private_key.pem
            openssl req -new -key node_private_key.pem \
              -subj '/CN=${cfg.serverName}' -out node.csr

            # Both roles on every leaf: a proxy authenticates as a client to the
            # game server, and the game server authenticates as a server to it.
            # One template is one thing to get wrong instead of two.
            openssl x509 -req -in node.csr -CA root_ca.crt -CAkey ca.key \
              -CAcreateserial -days 365 -sha256 \
              -extfile <(printf '%s\n' \
                'subjectAltName=DNS:${cfg.serverName}' \
                'extendedKeyUsage=serverAuth,clientAuth') \
              -out node.crt

            rm -f node.csr ca.key
            chmod 0440 node_private_key.pem
            chmod 0444 root_ca.crt node.crt
          '';
        };
      }
    ];
  };
}

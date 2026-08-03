# The VM holding the world. No public address: the proxies dial in over the
# group, and a VM outside the group has no route here at all.
{
  config,
  lib,
  hyperionGameServer,
  hyperionRules,
  hyperionReloadClient,
  buildStamp,
  ...
}: let
  port = 35565;
  # The name proxies reach this VM by, and therefore the name its certificate
  # has to be issued for. Group members resolve each other as
  # `<eastWest.hostName>.ix.internal`.
  fqdn = "${config.ix.networking.eastWest.hostName}.ix.internal";
in {
  hyperion.pki.serverName = fqdn;

  # One declaration of "this VM listens here", which is also what
  # `ix.endpointOf` reads on the proxy side.
  ix.networking.expose.hyperion = {
    inherit port;
    description = "hyperion game server, dialled by the proxies over the group";
  };

  ix.healthChecks.hyperion-game-server.unit = "hyperion-game-server.service";

  # hyperion opens its skin cache at the relative path `db/heed.mdb`, so it
  # writes to whatever the working directory is. The unit runs with
  # `ProtectSystem = strict` and no `WorkingDirectory`, which is `/` and
  # read-only, so it dies on EROFS before it listens. Point it at the state
  # directory systemd already creates for it. The cwd-relative path is
  # hyperion's to fix (ENG-10505); the working directory is the deployment's
  # to choose either way.
  systemd.services.hyperion-game-server = {
    serviceConfig.WorkingDirectory = "/var/lib/hyperion-game-server";
    # hyperion also caches the world it downloads under the XDG cache
    # directory, which the `directories` crate resolves from `$HOME`.
    # `DynamicUser` plus `ProtectHome` leaves that unset, `ProjectDirs::from`
    # returns None, and the server dies on an `.expect("failed to get AppId")`
    # that names the wrong thing. Same state directory, so both caches land
    # together.
    environment.HOME = "/var/lib/hyperion-game-server";
  };

  services.hyperion-game-server = {
    enable = true;
    package = hyperionGameServer;
    event = "smash";

    # The one store path that is allowed to move on a rules-only deploy. The
    # module puts it in `X-Reload-Triggers` and nowhere else, so a build that
    # changes only this reaches the running server as `systemctl reload` and
    # nobody is disconnected. Everything else -- a component's layout, the
    # engine -- moves `ExecStart` instead and is a restart, which is correct: a
    # system compiled against a layout the world no longer holds is memory
    # corruption rather than a stale build.
    rules = "${hyperionRules}/lib/${hyperionRules.dylibName}";
    reloadClient = hyperionReloadClient;

    buildStamp = {
      inherit (buildStamp) rev committedAt dirty;
    };

    inherit port;
    pki = {
      rootCaCert = "/var/lib/hyperion-pki/root_ca.crt";
      cert = "/var/lib/hyperion-pki/node.crt";
      privateKey = "/var/lib/hyperion-pki/node_private_key.pem";
    };
  };
}

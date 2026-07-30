# A VM players connect to. It terminates the Minecraft connection and forwards
# bytes to the game server without reading them, so this is the only part of
# the fleet meant to be reachable from outside it. Whether it currently is, is
# the `ipv4` comment below.
{
  config,
  hyperionProxy,
  ix,
  lib,
  nodes,
  pkgs,
  ...
}: let
  # Resolved from the game server's own `ix.networking.expose` declaration, so
  # a port change there reaches here without a second edit, and a typo in the
  # listener name is an evaluation error rather than a connection refused.
  game = ix.endpointOf nodes.hyperion-game "hyperion";

  # The full name, not the bare host and not an address. The proxy uses this
  # host as the TLS server name it expects on the game server's certificate,
  # so it has to be exactly the name that certificate was issued for.
  gameFqdn = "${game.host}.ix.internal";
in {
  hyperion.pki.serverName = "${config.ix.networking.eastWest.hostName}.ix.internal";

  ix = {
    networking = {
      # `ipv4 = true` is off because the address does not exist to be given,
      # not because a public address is unwanted. This fleet wants one: a Java
      # client resolves through the JDK, which prefers IPv4, and a large share
      # of players have no IPv6 at all.
      #
      # `15.204.22.192/26` is the region's only Additional IP block, and OVH
      # reports `routedTo.serviceName = null` for it -- routed to no vRack and
      # to no server, so it is delivered nowhere. Attaching it to the vRack is
      # the fix and it cannot be done today: all three vRacks report
      # `resource.state = "suspended"` and every vRack call answers HTTP 460,
      # "This service is expired", with no billing cause (active lifecycle,
      # automatic renew, $0.00 due). OVH ticket 713661, ENG-11229.
      #
      # Until that clears the block is worse than absent. A VM that takes an
      # address from it sources from the block, matches the host's
      # `from 15.204.22.192/26 iif br-north lookup 200` rule, and is handed the
      # dead gateway `15.204.22.254`, so everything it sends is blackholed.
      # Measured from inside a guest, not only from the host's routing table:
      # `vip-probe`, holding `15.204.22.195/32`, gets
      # `Destination Host Unreachable from 10.0.0.1` pinging `1.1.1.1`, while a
      # VM with no VIP answers in 0.7 ms. Both proxies did exactly this when
      # they held addresses -- unable to resolve, unable to substitute, unable
      # to finish their switch. ENG-10881.
      #
      # Do not check delivery by probing the gateway. OVH answers ARP for every
      # address on that segment, including `198.51.100.7`, which is TEST-NET-2
      # and belongs to nobody here. An ARP probe therefore succeeds whether or
      # not the block is delivered, so a check built on one cannot fail and is
      # worth nothing. `routedTo` at the provider, and the route table on the
      # host, are the facts that distinguish the two.
      #
      # So public ingress for this fleet comes from the host rather than from
      # this VM: a hil host carrying a proxy translates its own routed `bond0`
      # address on a Minecraft port to the proxy living on it, through
      # `services.ix.vmPublicIngress` in that host's inventory. Host addresses
      # work and need no provider action -- `15.204.109.254` answers a laptop
      # in 21 ms.
      #
      # What that costs, named here so the next reader can check it rather than
      # discover it:
      #
      #   - The public address becomes a property of the host. Move a proxy and
      #     a different address serves it.
      #   - The forward names a `vmId` and a `10.0.0.x` guest address, both of
      #     which are allocated at VM create. They exist only after this fleet
      #     has been applied once, and they go stale when a VM is recreated.
      #   - So the forward is not declared here and cannot be. This file
      #     describes the fleet; it cannot describe facts the fleet does not
      #     know until it has been deployed.
      #
      # A name in front absorbs the first two, with one record per
      # proxy-carrying host -- see below -- which this fleet needs anyway. The
      # third is real and unfixed: the fleet's public reachability is written
      # in a second place with nothing comparing the two copies.
      #
      # Revisit when OVH 713661 clears, but do not assume the answer flips. A
      # VM-owned address is the better shape when a client cannot be told a
      # port, because then the address follows the thing that moves. It is not
      # better here: SRV tells a Minecraft client the port (below), which
      # removes the only structural objection to sharing a host's address, and
      # one `/26` is 62 addresses for an entire region, which is a small-N
      # convenience rather than an ingress design.

      # Nothing declares a name in front of the port below, and the situation
      # is worse than a missing record: the names resolve, to the wrong region.
      # `*.ix.dev` is an apex wildcard pointing at `40.160.30.136`, a VIN host,
      # and asking Cloudflare's own nameservers rather than a resolver cache it
      # answers for every name this fleet would want -- `hyperion.apps.ix.dev`,
      # `mc.apps.ix.dev`, `play.ix.dev` -- and also for
      # `hil-compute-1.host.ix.dev` and `hil-compute-3.host.ix.dev`, the two
      # hosts that would carry the proxies. None has an A record of its own and
      # none has any AAAA. So a client told any of these today opens a
      # connection to the wrong region and finds nothing listening, which fails
      # as a timeout rather than as a name error. ENG-11218; the gate that has
      # to exist before a name can be claimed is ENG-11222.
      #
      # An explicit record beats the wildcard, and records are generated from
      # inventory in ix's `nix/terraform/cloudflare/dns-ix-dev.nix`, so the fix
      # is a derivation from inventory rather than a hand-added entry. The
      # shape below is decided rather than open, so it is recorded here instead
      # of being re-derived by whoever wires it up.
      #
      # It has an ordering dependency worth respecting rather than working
      # around: the service records below point at host names, and those host
      # names have certificate coverage but no records of their own. Publishing
      # a fleet name before the host names resolve would hand players a name
      # that resolves to the wrong region, which is the situation this note
      # exists to end rather than to spread.
      #
      # One name for the fleet, declared identically by every proxy -- never
      # one name per VM. A player's endpoint has to resolve to the *role*,
      # because any proxy will do; the record set is then the union of the
      # proxies' addresses, and replacing a proxy costs one record instead of
      # the endpoint. A name per VM would publish this fleet's topology into
      # people's server lists and make every scaling decision visible to them.
      #
      # For Minecraft the record set has to include `_minecraft._tcp.<name>`
      # SRV and not only an address record, and the reason decides whether the
      # port is a scarce resource: a Java client resolves SRV before A/AAAA, so
      # the name can point at any port and the player still types only the
      # name. Two fleets can share one edge address on 25565 and 25566 with
      # neither player noticing.
      #
      # The trap in that, which is the part nobody rediscovers cheaply: after
      # SRV resolution the client puts the SRV *target* in its handshake, not
      # the name it was given. Minecraft does carry a name in its first bytes
      # -- `Server Address` in packet 0x00, which is what Velocity routes
      # virtual hosts on -- but many names pointing at one shared target all
      # arrive identical, and the demultiplexing key is gone. So either one
      # address record per fleet name, or a distinct SRV target per fleet.
      # ENG-11227.

      # The one declaration of "players arrive here". Nothing outside the guest
      # reads it yet -- `ix apply` reads `groups` and `ipv4` and nothing under
      # `crates/` reads `expose` at all -- so today this opens the guest
      # firewall and nothing more.
      expose.minecraft = {
        port = 25565;
        description = "public Minecraft entrypoint";
      };
    };

    # Two checks, and they fail differently on purpose. The unit check says the
    # process died. The handshake check says the process is up and cannot
    # serve, and that is the twelve hours this fleet lost.
    healthChecks = {
      hyperion-proxy.unit = "hyperion-proxy.service";

      # `systemctl is-active` cannot see the failure that matters here.
      # hyperion-proxy binds its listener at startup and holds one long-lived
      # connection to the game server, multiplexing every player over it --
      # `ss -tn` inside a proxy shows exactly one ESTAB to the game port with
      # no players connected. The unit's state says nothing about that link. It
      # read `active (running)` for twelve hours while cross-host east-west was
      # down and nothing could have played.
      #
      # It is worse than a stale reading. When the backend read loop fails the
      # proxy signals every player task and drops every player socket, and a
      # re-dialled server would wait forever for an opening handshake that a
      # resumed connection never sends. So `active` means neither "a player can
      # join" nor "the players already here are still connected", and nothing
      # in the unit distinguishes those from healthy.
      #
      # This asks the question a player asks. The proxy does not answer a
      # status request itself, it forwards it -- the reply carries the game
      # server's own MOTD and player cap, which the proxy has no way to know --
      # so one request proves the listener is bound, the group name resolved,
      # the cross-host path carried the bytes, and the mutual-TLS handshake
      # against the game server succeeded. Grepping for `protocol` is enough
      # because any status document at all can only have come from the backend.
      #
      # Watched fail before being trusted, inside `hyperion-proxy-0`: with the
      # game server's port dropped by a temporary nftables rule, the unit check
      # passed and this one failed; with the rule removed, this one passed
      # again. It also fails when nothing is listening.
      #
      # The payload is a Minecraft handshake (packet 0x00, intent 1 = status)
      # followed by a status request, for host `127.0.0.1` port 25565. It is a
      # constant, so it ships base64 rather than as shell escapes -- a
      # `printf '\x10\x00...'` spelling of the same bytes is one backslash away
      # from a check that connects and asks nothing.
      hyperion-proxy-serves = {
        description = "the proxy answers a Minecraft status handshake from the game server";
        command = [
          (lib.getExe pkgs.bash)
          "-c"
          "echo EACIBgkxMjcuMC4wLjFj3QEBAA== | base64 -d | ${lib.getExe' pkgs.netcat-openbsd "nc"} -w 5 127.0.0.1 25565 | grep -qa protocol"
        ];
      };
    };
  };

  # `nc` is in the closure only because this file puts it there. The platform
  # pins a probe binary for the `http` and `tcp` check sugar, but an explicit
  # `command` gets its string context stripped when the fleet plan is rendered
  # (`planHealthChecks` in lib/image/fleet.nix), so nothing else retains what
  # the command names. Drop this line and the check above becomes a store path
  # that is not in the guest.
  environment.systemPackages = [pkgs.netcat-openbsd];

  services.hyperion-proxy = {
    enable = true;
    package = hyperionProxy;
    listen = "[::]:25565";
    # A name, not an address, and that is not a stylistic choice: the name
    # becomes the TLS server name the proxy expects on the game server's
    # certificate, which is issued for this exact string. Group members resolve
    # each other as `<name>.ix.internal` through the group's own resolver
    # (ENG-10855, fixed in ix#8978 and ix#9095), so nothing has to pin an
    # address to make this work. The `/etc/hosts` pin this file used to carry
    # for that is gone.
    #
    # Two typed fields rather than one `host:port` string, since hyperion#1078:
    # the string form made the port a second copy of the game server's own
    # option, free to drift from it.
    gameServer = {
      host = gameFqdn;
      inherit (game) port;
    };
    pki = {
      rootCaCert = "/var/lib/hyperion-pki/root_ca.crt";
      cert = "/var/lib/hyperion-pki/node.crt";
      privateKey = "/var/lib/hyperion-pki/node_private_key.pem";
    };
  };
}

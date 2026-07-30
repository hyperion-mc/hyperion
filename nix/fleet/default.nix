# The deployed fleet: one game server behind three proxies.
#
#   players -> hyperion-proxy-0 \
#   players -> hyperion-proxy-1  >-- hyperion-game (private, no public address)
#   players -> hyperion-proxy-2 /
#
#   nix build .#hyperion-game-system .#hyperion-proxy-0-system \
#             .#hyperion-proxy-1-system .#hyperion-proxy-2-system
#   ix apply .#hyperion-game .#hyperion-proxy-0 .#hyperion-proxy-1 .#hyperion-proxy-2
#
# Applied by hand. There is no CI deploy and no automatic apply, deliberately.
#
# ---------------------------------------------------------------------------
# WHY THIS LIVES HERE, AND WHY IT IS NOT ITS OWN FLAKE
# ---------------------------------------------------------------------------
#
# It used to live in index, at `examples/minecraft/hyperion`, as a nested flake
# with its own lock pinning `github:hyperion-mc/hyperion`. That made index
# import hyperion while hyperion already imported index -- a cycle broken only
# by the two locks, and the two locks are what let the halves drift apart.
#
# They did. hyperion#1078 changed `services.hyperion-proxy.gameServer` from a
# `host:port` string to a submodule; the consumer over in index kept passing a
# string; the pin could not advance at all and nobody found out for a day,
# because no single evaluation covered both sides. ENG-11448.
#
# Here there is no pin. The module and its consumer are in the same commit and
# one evaluation covers both, so that class of bug is not caught -- it is
# impossible.
#
# THAT IS ALSO WHY THIS IS NOT A NESTED FLAKE, and it is the thing to push back
# on when somebody proposes one because the root flake feels crowded. A
# `nix/fleet/flake.nix` would have its own lock, that lock would pin its own
# `index`, and the repo would once again hold two versions of index that can
# disagree -- which is precisely the shape that hid ENG-11448. The fleet is
# outputs of the root flake, on the root lock, with exactly one `index` node.
#
# Plain Nix rather than the `.ix` JavaScript syntax for a related reason:
# `importIxWasm` needs `builtins.wasm`, which only ix's patched Nix client has,
# and hyperion's CI installs stock Nix. Verified on stock nix 2.34.8
# (`builtins ? wasm` is false there): this file evaluates, and the four node
# derivations come out byte-identical to what the `.ix` form produced.
{
  index,
  # The x86_64-linux binaries the guests run. Always x86_64-linux: these are
  # Linux guests whatever machine types `nix build`, which contributes a
  # builder rather than an identity.
  guestPackages,
  # This repo's own service modules. No input, no pin -- that is the point.
  nixosModules,
}:
index.lib.mkFleet {
  # One private segment. A VM outside it has no route to the game server,
  # which is the only thing keeping an unproxied client off the world.
  defaults = [
    {ix.networking.groups = ["hyperion"];}
    {
      _module.args = {
        hyperionGameServer = guestPackages.smash;
        hyperionProxy = guestPackages.hyperion-proxy;
      };
    }
    ./pki.nix
    nixosModules.game-server
    nixosModules.proxy
  ];

  nodes = {
    "hyperion-game".modules = [./game.nix];

    # Interchangeable, and `replicas` is how that is said rather than implied:
    # three copy-pasted node entries would let one drift from its siblings.
    # Raising the digit adds a `-system` attr and an apply target, not a node
    # definition.
    #
    # What it does not buy, because it cannot: nothing here places the proxies
    # on different hosts. `ix apply` has no way to ask and the scheduler takes
    # the host reporting the most free memory, so one apply can land every
    # proxy on one host. ENG-11225.
    "hyperion-proxy" = {
      modules = [./proxy.nix];
      replicas = 3;
    };
  };
}

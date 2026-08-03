# A machine to build hyperion on, declared in the fleet rather than kept beside
# it. It serves nothing, no player reaches it, and the game does not depend on
# it -- it exists so that "where do I build this" has the same answer as "where
# is this deployed", written in one file a reviewer can read.
#
# The loop it is for is in README.md ("A dev machine in the fleet"): apply it,
# `ix shell` in, edit and build on a Linux box that already has the platform's
# caches, push over your own forwarded credentials.
#
# ---------------------------------------------------------------------------
# WHAT THIS FILE DELIBERATELY DOES NOT DO
# ---------------------------------------------------------------------------
#
# Most of a dev box is already answered by the platform, and restating an
# answer is how two copies of it start to disagree. Named here so the next
# reader can check the claim rather than re-derive it, and so nobody adds a
# second copy believing it was missing:
#
#   - Nix is already configured for this repository. `lib/image/platform.nix`
#     sets `experimental-features` to a list including `flakes`, `nix-command`
#     and `ca-derivations`, and `modules/profiles/base` puts `cache.ix.dev` in
#     `substituters` with its `ix-workspace:` key trusted. That is exactly what
#     this flake's own `nixConfig` block asks of the machine evaluating it, so
#     a guest copy of those settings would buy nothing and could drift.
#
#   - git is already installed, for every user rather than for one profile
#     (`modules/profiles/base`, which also leaves a fallback identity so a
#     first `git commit` in a fresh VM does not die on a missing email).
#
#   - The working directory is already a convention: `/work/ix`, from
#     `ix.profiles.base.shellWorkspace.directory`. The platform pre-creates it
#     with a tmpfiles rule and login shells cd into it, so a second directory
#     declared here would be one the shell does not land in. The checkout goes
#     there.
#
# NOTHING CLONES THE REPOSITORY INTO IT, and that is a choice rather than an
# omission. A clone is state, not configuration: activation would have to pick
# a revision, and every later `ix apply` would then either fight the working
# copy or silently leave it alone, which is a worse contract than having no
# opinion at all. Reading this repository needs no credentials -- it is public
# -- but pushing needs the developer's own, and those are deliberately theirs
# and not the VM's. So: `git clone` once, by hand, into `/work/ix`.
#
# THE `ix` CLI IS NOT HERE, and cannot be added from this repository today.
# index packages no `ix` binary (its `packages/` tree has `ix-fleet`,
# `ix-credential`, `ix2nix`, and no CLI), and the CLI's own repository is
# private: an unauthenticated `GET /repos/indexable-inc/ix` answers 404 while
# the same call for `indexable-inc/index` answers 200. hyperion is public and
# its CI installs stock Nix, so a private flake input would break evaluation
# for every outside contributor and for the gate. The consequence to plan
# around: `ix` commands are typed on your machine against this VM, never from
# inside it, so anything needing the CLI in the guest -- a nested VM, an `ix
# apply` from the dev box -- is out of reach here. ENG-12081.
#
# THERE IS NO RAM OR DISK KNOB TO SET, which is why this file sets none. A
# fleet node takes `modules`, `deployment`, `tags`, `groups`, `dependsOn`,
# `replicas` and `updateStrategy` (`lib/image/fleet.nix`) and none of those
# describes a machine size; neither `ix apply` nor `ix new` carries a sizing
# flag either. Sizing belongs to the platform, and measured from inside a node
# of this fleet it is not the binding constraint anyway:
#
#     $ ix shell hyperion-game -- sh -c 'grep MemTotal /proc/meminfo; nproc'
#     MemTotal:       268435456 kB
#     64
{
  config,
  lib,
  hyperionDevEnvironment,
  ...
}: {
  # Its own east-west segment, NOT the fleet's. `default.nix` puts every node in
  # `hyperion`, and that group is the only thing keeping an unproxied client off
  # the game server -- so the one box in this fleet whose purpose is running
  # code nobody has reviewed yet is the one box that belongs outside it. The
  # loop this node serves is build-and-push, which needs no route to the game.
  #
  # `mkForce` because `groups` is a `listOf str`: without it the fleet default
  # merges in rather than being replaced. The slug is get-or-created under the
  # deploying user at create, so it needs no separate setup.
  #
  # Groups are joined AT CREATE (`lib/image/platform.nix`), like `ipv4`, so this
  # is not something a re-apply changes on a VM that already exists: moving this
  # node between segments is `ix rm` and apply again.
  ix.networking.groups = lib.mkForce ["hyperion-dev"];

  # `./pki.nix` is a fleet default and gives `serverName` no default, so every
  # node has to answer it -- including one that runs neither hyperion service.
  # Answering it and then not minting is the honest pair: the unit exists to
  # hand the two services their mutual-TLS material, nothing here reads that
  # material, and the authority is the public throwaway committed next to this
  # file, so a certificate minted here would attest to nothing.
  hyperion.pki.serverName = "${config.ix.networking.eastWest.hostName}.ix.internal";
  systemd.services.hyperion-pki.enable = false;

  # The build environment, taken from the flake's own `devShells.default`
  # rather than restated: `hyperionDevEnvironment` is the same binding that
  # shell installs, so `nix develop` on a laptop and a login shell on this VM
  # cannot disagree about which compiler builds hyperion. The channel under it
  # is `rust-toolchain.toml`, which is where the version lives for rustup users
  # and for CI too.
  #
  # x86_64-linux for the reason `guestPackages` is: this is a Linux guest
  # whatever machine types `nix build`, which contributes a builder rather than
  # an identity.
  environment = {
    systemPackages = hyperionDevEnvironment.packages;
    # What rust-analyzer resolves `std` sources through. The devShell exports
    # the same string; a shell with the compiler but not this one gives
    # go-to-definition that lands nowhere.
    variables.RUST_SRC_PATH = hyperionDevEnvironment.rustSrcPath;
  };

  # No health check, deliberately. The two service nodes declare theirs because
  # a process can be running and unable to serve, a distinction only a probe
  # makes. Nothing here serves: this node is ready when it boots, and a check
  # re-asking a compiler for its version every interval would be probing a
  # store path that cannot change without a switch.
}

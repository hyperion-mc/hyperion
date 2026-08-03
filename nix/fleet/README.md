# hyperion: one game server, three proxies

This is the deployment, in the repository whose game it deploys. It used to be
an example in index (`examples/minecraft/hyperion`), a nested flake with its own
lock pinning `github:hyperion-mc/hyperion` -- so index imported hyperion while
hyperion already imported index, a cycle broken only by two locks. Two locks are
what let the halves drift: hyperion#1078 changed an option's type, the consumer
in index kept passing the old one, and the pin could not advance for a day
because no single evaluation covered both sides (ENG-11448). Here there is no
pin, so that class is impossible rather than merely caught.

`nix/fleet/default.nix` carries the longer version, including why this must not
become a nested flake the next time the root flake feels crowded.

```
players -> hyperion-proxy-0 \
players -> hyperion-proxy-1  >-- hyperion-game (private, no public address)
players -> hyperion-proxy-2 /
```

```sh
nix build .#hyperion-game-system .#hyperion-proxy-0-system .#hyperion-proxy-1-system .#hyperion-proxy-2-system
ix apply .#hyperion-game .#hyperion-proxy-0 .#hyperion-proxy-1 .#hyperion-proxy-2
```

Change anything and run the same command again. Each VM is reused by name and
switched in place, so only the units whose definition changed restart.

## What the fleet cost in the last 24 hours

```sh
python3 nix/fleet/spend.py            # dollars per VM + fleet total, last 24h
python3 nix/fleet/spend.py --since 7d
```

It reads `ix billing usage --since ... --json` and filters the per-VM
`resource_spend` rows to names starting with `hyperion-`, so it needs a
logged-in `ix` and a provisioned billing account; without the account it
repeats ix's own error and exits nonzero instead of printing zeros.

## Build the fleet once, not once per VM

The `nix build` line is what makes the apply finish in minutes instead of
hours, and it is the only reason for the `packages` output in `flake.nix`.

Without it, `ix apply` hands each VM the derivation and each VM realises it in
its own store. The nodes here are one closure with several hostnames; measured
over the three-node fleet:

    union of the three systems   1766 store paths, 7.51 GB
    any one of them              7.50 GB

So three guests compile the same 7.5 GB three times, from a store that starts
with only what the base image carries, over a link to two public caches that do
not serve a Rust workspace from someone else's flake. That is the 83-minute
apply that never finished (ENG-10800, ENG-10839).

Build it first and the numbers are different, because the work happens once, on
a machine that is a builder:

    nix build, all three nodes    195s (78 derivations, one remote builder)
    VM create, per node           1.0s to 1.7s, "reusing golden snapshot"

`ix apply` then finds the system already in your store and exports the closure
instead of asking the guest to produce it (`SwitchTarget::LocalClosure`): the
guest answers with the store paths it lacks and only those cross. No guest
compiles anything.

Where `nix build` runs is your choice and nothing about ix makes it: it is your
nix, your `/etc/nix/machines`. On a Mac it has to be a remote builder, because
these are `x86_64-linux` systems. `ix apply` itself still never builds and
never substitutes; if the paths are not already there, it behaves exactly as
before.

What this does not fix: the closure crosses your uplink once per VM. Nodes
sharing one closure still pay one export each, so the third proxy costs another
one. The shape that avoids that is one push into the region's CAS and N VMs
materializing from it, which is what ENG-10839 is about.

## Why the game server has no public address

Proxies dial the game server, not the other way round. So the game server
needs one address its proxies can reach and nothing else, and it lives only on
the `hyperion` east-west group. A VM outside that group has no route to it,
which is the only thing keeping an unproxied client off the world.

The proxy count is one digit in `default.ix`: `replicas: 3`. `mkFleet` expands
that into `hyperion-proxy-0` through `hyperion-proxy-2`, which is why the
proxies are one spec rather than three copy-pasted node entries -- with three
entries their interchangeability is a comment, and one of them can be edited
alone. The game server does not change when the count does: it has supported
several connected proxies since hyperion#940.

## Why the proxy names the game server rather than addressing it

`--server` takes a `host:port`, and the host becomes the TLS server name the
proxy expects on the game server's certificate. An address there fails the
handshake against a certificate issued for a name, and the failure reads as a
connection problem rather than a naming one. Group members resolve each other
as `<name>.ix.internal`, so that is the string on both sides.

That resolution now works, which is new. This file used to document an
`/etc/hosts` pin (`hyperion.gameAddress`) that mapped the name to an address to
keep the name while skipping the resolver. ix#8978 gives each group a
`<prefix>::1` gateway and ix#9095 makes ix-dns bind an IPv6 listener on it, so
a member's query carries a source address the resolver classifies into its own
group's view (ENG-10855). Measured inside `hyperion-proxy-0`:

    $ head -1 /etc/resolv.conf
    nameserver fd00:1:1f:856f::1
    $ getent hosts hyperion-game.ix.internal
    fd00:1:1f:856f:4250:a3a5:16b7:d9ef hyperion-game.ix.internal

The option and the `networking.hosts` block it fed are deleted.

## The certificate authority in this directory is public

`dev-ca.key` is committed. Every node installs it, mints its own leaf, and
deletes it. The key is in this repository and in the nix store, and anyone
holding it can impersonate a proxy and take commands to the world.

**The justification for that has expired, and the move is what expired it.**
When this lived in index's example tree the sentence here read "anything using
it is a demonstration rather than a deployment", and that was true: it was a
worked example that happened to run. It is now the deployment, in the game's own
repository, and its only job is the live server -- so "it is public because this
is a demonstration" is no longer available as a reason.

Exposure did not change. Both repositories are public and the live fleet has
been running on this key for days; moving the file moved nothing. What changed
is that there is no longer a story under which this is fine.

Tracked separately as its own decision rather than bundled into the move. For
anything real, deliver a key out of band and point `hyperion.pki.caKeyFile`
at it.

One shared secret rather than three certificates: every node gets the same CA
key and mints its own leaf at activation, so nothing has to copy a certificate
from one VM to another.

## Nothing here can spread the proxies across hosts

Three proxies are three failure domains for the proxy process and one for the
hardware under them, and this directory cannot change that. Measured:

    $ ix ls --output json | jq -r '.[] | "\(.name)  \(.node)"'
    hyperion-proxy-1  hil-compute-1
    hyperion-proxy-0  hil-compute-1
    hyperion-game     hil-compute-3

Both proxies on one host. That is not bad luck. `select_by_available_memory`
orders candidate hosts by the `available_memory_mib` their last heartbeat
reported and reserves nothing against a placement it has just made, so every VM
in one `ix apply` goes to whichever host was ahead at that instant. There is no
anti-affinity concept to declare and no host to name -- `ix apply` has no flag,
and the fleet node spec has no key. ENG-11225.

So read the redundancy claim narrowly: a proxy crashing, or being switched to a
new system, leaves the others serving. A host failing may take all of them. The
honest fix is a spread key the scheduler honours; a `--node` flag would only
move the problem into whoever types the command.

**The rule is deterministic, so it can be predicted even though it cannot be
asked.** Before creating the third proxy, the three hosts reported:

    hil-compute-1  MemAvailable 373963 MiB   <- carries proxy-0 and proxy-1
    hil-compute-2  MemAvailable 313773 MiB
    hil-compute-3  MemAvailable 389715 MiB   <- highest, so this should win

Written down before the outcome so the record shows a prediction rather than a
rationalisation. **It was then re-measured immediately before the create, and it
had inverted:**

    host             recorded 14:0x   measured 15:01, just before the create
    hil-compute-1    373963 MiB       399285 MiB   <- now highest
    hil-compute-2    313773 MiB       339849 MiB
    hil-compute-3    389715 MiB       371943 MiB   <- was the prediction

An hour apart, with no workload anyone started or stopped in between: the two
hosts simply swapped on background noise. The second prediction was recorded
before the create too, and it was the one that came true -- `hyperion-proxy-2`
landed on hil-compute-1 with the other two.

That inversion is the finding, and it is worth more than the good outcome would
have been. **A property that flips on memory noise between two hosts nobody is
choosing between is not a property the fleet has** -- it is one it may happen to
get. Landing on hil-compute-3 would have looked like success and hidden that
entirely. Anti-affinity being inexpressible in the fleet spec, in `mkFleet`, and
on `ix apply` is the thing to fix; ENG-11225 is where it lives.

`ix migrate` can move a VM off its host afterwards, and it chooses the
destination itself, excluding the source. That is a lever, not a fix: **a manual
correction of a placement nobody could ask for is not the design meeting the
requirement.** Anyone reading this and seeing proxies on two hosts should not
conclude the fleet can express that. It cannot.

hil-compute-2 is worth avoiding for two reasons rather than one: it has not
taken the east-west fix yet, and it is the host missing the group DNS gateway
(ENG-11226). A member placed there fails twice over. The only thing making that
unlikely today is that it also has the least free memory, which is not a reason
anyone chose.

## There is no name in front of the endpoint

A player is handed an address, so every correct fix to where this fleet lives
costs them a new one. ENG-11218 carries that; the claiming gate that has to
exist before any name can be handed out is ENG-11222.

The shape the fleet should declare is written at length beside
`expose.minecraft` in `proxy.nix`, because it is decided rather than open. The
two load-bearing parts:

- **One name for the fleet, on the proxy role, never one per VM.** The player's
  endpoint must resolve to "a proxy", so the record set is the union of the
  proxies' addresses and replacing a proxy costs a record instead of an
  endpoint.
- **`_minecraft._tcp.<name>` SRV, not an address record alone.** A Java client
  resolves SRV first, so the name can point at any port while the player still
  types only the name -- which means a public Minecraft port is not the scarce
  per-host resource it is usually treated as. The catch is that after SRV
  resolution the client sends the SRV *target* in its handshake, so several
  names behind one shared target arrive indistinguishable and name-based
  demultiplexing at an edge stops working. ENG-11227.

## State of the deployment, 2026-07-29

Four VMs exist in `us-west-1`. `hyperion-game` on hil-compute-3, and
`hyperion-proxy-0`, `-1` and `-2` all on hil-compute-1.

**The third proxy exists, and it is the only machine in the fleet running the
current build.** Not because it was updated -- because it was *created*. Every
existing node failed to switch (below), so the one node that never had to switch
is the one that is current. That is the same asymmetry that caused the outage,
seen from the good side.

**The fleet still does not have the property the user asked for.** All three
proxies are on one host, so a hil-compute-1 fault takes every entrypoint.
Placement is not something this spec can express; see above. The only remaining
lever is `ix migrate`, which is a deliberate action to take cold rather than at
the end of an incident.

### The 13m40s outage, 15:04:28Z to 15:18:08Z

Applying the game-version bump to all four nodes took the public endpoint down
for **13 minutes 40 seconds**, measured at 2 s cadence: last success 15:04:28,
first failure 15:04:30, last failure 15:18:02, first success 15:18:08, 136
consecutive failed probes.

Three of four switches failed, identically:

    ✗ hyperion-game     switch-to-configuration exited with status 1
    ✗ hyperion-proxy-0  switch-to-configuration exited with status 1
    ✗ hyperion-proxy-1  switch-to-configuration exited with status 1
    ✓ hyperion-proxy-2  ready          <- created, not switched

**The switch stopped the thing it was standing on.** From the game server's
journal, and byte-for-byte the same on proxy-1 one second later:

    15:04:29 nixos[25901]: switching to system configuration /nix/store/dvspfsvzm...
    15:04:29 systemd[1]: Stopped target Local File Systems.
    15:04:29 systemd[1]: Stopping D-Bus System Message Bus...
    15:04:29 systemd[1]: Unmounting /tmp...
    15:04:29 systemd[1]: Unmounted /tmp.
             (silence)

Diffing the running system against the target system found half the cause in one
comparison: `tmp.mount` is present in the old system and **absent** in the new
one, and `switch-to-configuration` correctly stops a unit the new configuration
no longer declares.

The other half took a reproduction to find, and it is not the `local-fs.target`
cascade this file first blamed. It is one line in nixpkgs. From the unit systemd
actually loaded, in a clean NixOS VM with no ix image involved:

    # systemctl show dbus-broker.service -p Requires -p RequiresMountsFor -p WantsMountsFor
    Requires=tmp.mount
    RequiresMountsFor=/tmp
    WantsMountsFor=/tmp /var/tmp

`nixos/modules/services/system/dbus.nix` puts `RequiresMountsFor = [ "/tmp" ]`
on the system bus, under a comment asking for **ordering**: "We get errors when
reloading the dbus-broker service if /tmp got remounted after this service
started". But `RequiresMountsFor=` is `Requires=` plus `After=`, so it also
tells systemd to stop the bus whenever it stops the mount.
`switch-to-configuration` is issuing that stop over that bus and is blocked
waiting for it, so it loses its own connection, exits 1, and leaves the services
it already stopped stopped.

The ordering that comment wanted was never at risk. `WantsMountsFor=/tmp
/var/tmp` in the same output is `PrivateTmp=`'s contribution and carries the
same `After=`, so the nixpkgs line was supplying nothing but the kill switch.
`modules/system/dbus-survives-mount-removal.nix` drops it to
`WantsMountsFor=`, and `tests/switch-stops-a-mount-vm.nix` runs a real switch to
prove the machine survives one -- the first check in this repo that runs a
switch rather than reading the generation it would build.

That is the catastrophic half fixed, not the whole thing. With the bus alive the
same switch reaches activation, repairs `/tmp`, and records the new generation,
but still exits **4** on `Failed to stop tmp.mount` when anything holds /tmp
open, and the node agent reads any nonzero as a failed apply. Closing that needs
the generation *before* a mount is retired to carry `X-StopOnRemoval = false` on
it, because `switch-to-configuration` reads that key from the **running**
generation and does its `daemon-reload` after the stop phase, so nothing shipped
later can reach it. ENG-11080 has the measurements.

The cause is index commit `dac64977`, *"image: keep /tmp on the rootfs instead
of a tmpfs sized by the boot base"*. **That commit is correct and should not be
reverted.** `boot.tmp.useTmpfs = true` mounted /tmp with `size=50%`, the kernel
resolves that percentage once at mount time against `totalram_pages()`, and an
ix guest mounts /tmp while only the 3 GiB unpluggable virtio-mem base exists --
giving a measured 1.42 GiB /tmp on a guest reporting 256 GiB, one of them
already full. This is a **missing migration, not a regression**: removing a
mount unit is not a safe in-place transition, and nothing noticed that.

ENG-11080, urgent. The property it asks for is one sentence: *a switch must never
stop a unit the switch itself depends on.* It is also checkable statically, from
the two systems' unit graphs, before any deploy -- which would be a better gate
than a VM test, because it can run on every apply rather than on every commit.
Nothing does that yet.

Three things worth keeping from how this failed:

- **Nothing was half-migrated.** All three failed nodes stayed on their old
  generation; `/run/current-system` never moved. That is the deploy tool
  behaving correctly under failure, and it is why recovery was
  `systemctl start hyperion-game-server` -- the unit was merely stopped, because
  the switch never got as far as replacing it. The reflex after a failed deploy
  is to assume a mixed state; here that reflex would have been wrong and
  expensive.
- **The guard for this was written today and the deploy that needed it is the
  deploy that failed to install it.** `hyperion-proxy-0` sat `active` with a
  dead backend for the whole thirteen minutes -- exactly the state the handshake
  check exists to catch -- and the check could not fire, because its own switch
  failed. It is deployed on `hyperion-proxy-2`, the node that was created.
- **`ix apply` writes nothing to stdout without a tty.** The log was empty for
  ninety seconds and looked stalled. The real signal is the JSONL trace under
  `~/.ix/trace/`. Worth knowing before scripting an apply.

**The game version bump is deferred, not done.** The lock change is merged and
correct; the fleet cannot receive it by the supported path until ENG-11080 is
fixed. Do not retry the apply before then -- it will fail identically, because
the unit diff is a property of the two closures rather than of timing.


**The fleet serves Minecraft, across hosts, through either proxy.** A real
status handshake -- protocol 776, the same packet a client sends -- against
each proxy returns the game server's own status:

    $ ix shell hyperion-game -- sh -c 'python3 /tmp/mcping.py \
        hyperion-proxy-0.ix.internal 25565'
    version:     {'name': '26.2', 'protocol': 776}
    players:     {'max': 12000, 'online': 0, 'sample': []}
    description: Getting 10k Players to PvP at Once on a Minecraft Server to
                 Break the Guinness World Record

Identical through `hyperion-proxy-1`. The path crosses hosts twice --
hil-compute-3 to hil-compute-1 to reach the proxy, hil-compute-1 back to
hil-compute-3 for the game server -- so this exercises the whole design: group
name resolution, the VXLAN data path, and the proxy's mutual-TLS link to the
game server.

`mcping.py` is in this directory. Run it from inside the group, where the names
resolve; it speaks the protocol rather than asking systemd for an opinion, which
is the difference that matters.

**That difference is now a health check rather than a warning.** The fleet used
to declare only `ix.healthChecks.hyperion-proxy.unit`, and that check could not
fail in the case that mattered. hyperion-proxy binds its listener at startup and
holds one long-lived connection to the game server, multiplexing players over
it; `ss -tn` inside a proxy shows a single ESTAB to the game port with nobody
playing. The unit's state says nothing about that link, so it stayed
`active (running)` for twelve hours while nothing could have played -- and when
the link does break the proxy drops every player socket and cannot usefully
re-establish, so `active` means neither "a player can join" nor "the players
already here are still connected".

`proxy.nix` now declares a second check that sends the same handshake to the
proxy's own listener. Both were run side by side inside `hyperion-proxy-0`, with
the game server's port dropped by a temporary nftables rule:

    backend blocked:   unit check PASS  <- blind
                       handshake  FAIL  <- fires
    rule removed:      handshake  PASS

It also fails when nothing is listening. That is the guard watched failing
before being trusted, which is the only way to know a guard is one.

The same test doubles as a check that the game server is not directly playable,
which is the property the private segment and the client certificate exist for.
Pointed at `hyperion-game:35565` it fails, and the raw bytes say why -- a
plaintext handshake gets exactly seven back:

    15 0303 0002 02 32

A TLS record of type 21, alert, fatal, `decode_error`. The game port speaks TLS
and refuses anything else, so reaching that port is not the same as being able
to use it. Through the script the same thing reads as
`expected status response (packet 0), got packet 3`.

**Public ingress now exists and works.** A handshake from a laptop against
`15.204.111.75:25565` -- no tunnel, no group membership -- returns the game
server's status. The path is internet, hil-compute-1's own routed address, DNAT,
proxy, cross-host, game server. Every hop from a player's client to the world is
now exercised.

That address is a host's, not the fleet's, for the reasons under `ipv4` in
`proxy.nix`: the region's ingress block is undelivered and its vRack suspended
(ENG-11229). So the endpoint is a host address with no name in front of it, and
both of those are still open.

Two things this run corrected that were true this morning:

- **Cross-host east-west works now.** It did not. `hyperion-proxy-0` could
  reach `hyperion-proxy-1` on the same host in 0.24 ms and got
  `Address unreachable` for the game server one host over. ix#9073
  (ENG-10976, ENG-11067) landed and the same ping is now 3/3 at 0.33 ms.
- **A `systemctl` reading of `active` proved nothing while it was broken.**
  hyperion-proxy binds its listener at startup and dials the game server per
  connection, so a completely unreachable backend was invisible to systemd.
  The unit crash-looped when the *name* would not resolve and went quiet when
  only the *path* was broken, which is the wrong way round. The handshake above
  is the check that would have caught it; nothing in the repo runs it
  (ENG-10986).

Fixed since this example landed:

1. **Groups can be created, a first apply joins them, and members resolve each
   other.** `ix group create` failed because `vm_groups` is a regional table
   with a foreign key into globally owned identity (ix#8841). `ix apply` reads
   `ix.networking.groups` off the evaluated system, so the group and its
   membership come up on a first apply from nothing. Resolution inside the group
   is ENG-10855, closed by ix#8978 and ix#9095 (see above).
2. **The game server reaches `active`.** Four faults, all fixed in #4246: the
   PKI material was unreadable by the two `DynamicUser` services that need it,
   the world database opened relative to a read-only working directory, `$HOME`
   was unset so the world cache had nowhere to go, and the listen address was
   built by string concatenation.
3. **The listen address is a `SocketAddr`.** hyperion#990 gives both events one
   launcher, so `--ip ::` reaches the socket instead of panicking.
4. **A guest no longer has to build.** With the systems built first, each switch
   goes straight to `importing closure`; nothing compiles in a VM. The three
   faults this file used to list as "properties of one artifact" (ENG-10487
   `nix-daemon.socket`, ENG-10512 store paths owned by nobody, ENG-10522 the
   14 GB root) were all about a guest trying to build. None of them appeared.

Open:

- **`ix apply` still cannot update the three guests whose generation declares
  `tmp.mount`, and the answer is to recreate them.** ENG-11080 (ENG-11315 is
  the same defect). Detail above; the bus half is fixed by
  `modules/system/dbus-survives-mount-removal` and gated by
  `tests/switch-stops-a-mount-vm.nix`, but a fix in the new generation cannot
  reach a guest that has not taken it -- switch-to-configuration reads the
  RUNNING generation's unit files and reloads only after its stop phase.

  Four runtime mitigations were measured on `hyperion-proxy-1` on 2026-07-29
  and all four fail. A `/run` drop-in setting `RequiresMountsFor=` empty does
  not drop the edge; nor does it after `systemctl stop dbus-broker.service`,
  nor after `systemctl daemon-reexec`, nor renamed to sort after nixpkgs' own
  `overrides.conf`. `umount -l /tmp` beforehand does not survive either: the
  running generation still declares the mount and `local-fs.target` still
  wants it, so it is back before the switch reaches its stop phase. The apply
  then failed exactly as before, and recovery was
  `systemctl start hyperion-proxy.service`.

  **Recreate is not a workaround here, it is the good state.**
  `hyperion-proxy-2` was created rather than switched, and on it
  `tmp.mount` has no `FragmentPath` at all -- it is synthesised from
  `/proc/self/mountinfo` -- so `dbus-broker` shows
  `Requires=dbus.socket system.slice` and nothing about /tmp. systemd only
  adds the Requires half of a mount dependency for a mount unit that has a
  fragment (`unit_add_mount_dependencies()`, src/core/unit.c; the `After=`
  edge is unconditional). A recreated guest keeps its tmpfs `/tmp`, gets the
  target build, and is out of the way of this permanently.

- **`dac64977` did not do what its commit message says, and guest `/tmp` is
  still a tmpfs with the identical frozen cap.** ENG-11365. The mount is made
  by ix's injected PID 1, before systemd exists at all:
  `crates/vm/guest/remote-bootstrap/src/main.rs:257` lists
  `("tmpfs", "/tmp", "tmpfs")` in `ESSENTIAL_MOUNTS`, with no size option, so
  the kernel's 50%-of-RAM default is resolved once against the 3 GiB
  virtio-mem boot base and frozen -- which is exactly the failure `dac64977`
  set out to remove. Measured, one mount each, no stacking:

      hyperion-game      /tmp tmpfs size=2172796k    MemTotal 256G
      hyperion-proxy-1   /tmp tmpfs size=1966976k    MemTotal 256G
      hyperion-proxy-2   /tmp tmpfs size=1491836k    MemTotal 256G

  So index#4332's ENOSPC is unfixed, `boot.tmp.useTmpfs` does not control
  whether a guest has a tmpfs `/tmp`, and the comment `dac64977` added to
  `lib/image/platform.nix` ("/tmp lives on the rootfs, like a normal
  machine") describes behaviour no guest has. What `useTmpfs` still controls
  is whether the GENERATION declares a `tmp.mount` unit -- which is the whole
  of this bug, because systemd adopts the existing mount under that unit and
  a switch that removes the unit unmounts it.

  Corrects an earlier version of this entry that said the pinned index rev
  sets `useTmpfs = true`. It does not; the value is `lib.mkDefault false`.
  Getting that wrong took reading the wrong `index` node: the old nested
  flake's lock carried TWO, the root-mapped `index_2` and hyperion's own
  transitive `index`. That trap is gone with the nested flake -- this repo's
  lock has exactly one `index` node -- and it is one of the reasons the fleet
  moved here.

- **A transitional generation is constructible, so the upgrade needs no
  destructive step.** Verified by eval rather than argued: `useTmpfs` is
  `lib.mkDefault false`, so one plain definition in the fleet's `defaults`
  wins, and both properties then hold at once -- `tmp.mount` declared, and
  `modules/system/dbus-survives-mount-removal` in effect.

      apply 1  index input >= 66c0bcca, plus { boot: { tmp: { useTmpfs: true } } }
      apply 2  drop that line

  Apply 1 removes no unit, so nothing is stopped; mount units are reloaded
  rather than restarted by switch-to-configuration and `dbus-broker` is
  `reloadIfChanged`, so the bus is not bounced. Apply 2 then retires the
  mount with the fix already in the RUNNING generation.

  Note what the currently pinned target is until that happens: `57962328`
  carries `dac64977` but **not** the bus fix (which landed at `66c0bcca`,
  15:09 the same day), and its built system still has
  `dbus-broker.service.d/overrides.conf` containing `RequiresMountsFor=/tmp`.
  Applying it to an existing guest is the landmine, not a step away from it.
- **Public ingress. This is the only thing left between the fleet and a
  player.** The region's one Additional IP block, `15.204.22.192/26`, is
  delivered nowhere: OVH reports `routedTo.serviceName = null` for it. The fix
  is attaching it to the vRack, and that is refused because all three vRacks
  report `resource.state = "suspended"` and every vRack call answers HTTP 460,
  "This service is expired" -- with no billing cause. OVH ticket 713661,
  ENG-11229, ENG-10881. Until then, taking an address from the block is worse
  than having none: `vip-probe`, a scratch VM holding `15.204.22.195/32`, gets
  `Destination Host Unreachable from 10.0.0.1` pinging `1.1.1.1`, while a VM
  with no VIP answers in 0.7 ms.

  The path that works needs no provider action: a DNAT from a hil host's own
  routed `bond0` address to the proxy on it, via `services.ix.vmPublicIngress`
  (ENG-11132). Host addresses route -- `15.204.109.254` answers a laptop in
  21 ms. `proxy.nix` explains why that is the right shape here rather than only
  the available one, and what it costs.

  One warning if you go looking: **no ARP probe can tell delivered from
  undelivered.** OVH answers ARP for every address on that segment, including
  `198.51.100.7`, which is TEST-NET-2 and belongs to nobody here. A check built
  on the gateway answering cannot fail.
- **The name resolves to the wrong region.** ENG-11218, gated on ENG-11222.
  Not merely missing: `*.ix.dev` is an apex wildcard pointing at
  `40.160.30.136`, a VIN host, and asked of Cloudflare's own nameservers it
  answers for `hyperion.apps.ix.dev`, `mc.apps.ix.dev`, `play.ix.dev`, and also
  for `hil-compute-1.host.ix.dev` and `hil-compute-3.host.ix.dev` -- the hosts
  that would carry the proxies. None has an A record of its own, none has any
  AAAA. So a client told any of these opens a connection to the wrong region
  and hangs. Explicit records win over the wildcard and are generated from
  inventory in ix's `nix/terraform/cloudflare/dns-ix-dev.nix`; shape in
  `proxy.nix`. The host names have to resolve before a fleet name pointing at
  them means anything, so this is ordered rather than parallel.
- **A group's DNS gateway is not consistently present.** ENG-11226. On the
  `hyperion` group's bridge, `fd00:1:1f:856f::1` is on hil-compute-1 and
  hil-compute-3 and absent on hil-compute-2, which carries two members of the
  same group. It has stopped being latent now that cross-host traffic works: a
  member placed on hil-compute-2 has no in-prefix resolver on its own host.
- **The proxies may all be on one host and nothing can ask otherwise.**
  ENG-11225, above.
- **A fleet cannot boot its own published image.** ENG-10839. `mkFleet` renders
  one per node at `packages.<node>`, and nothing outside the private ix repo can
  build it, which is why the export cost above is per-VM rather than a single
  push into the region's CAS.
- **The fleet is evaluated by CI, but nothing is enforced.**
  `checks.<system>.fleet-eval` forces all four nodes' toplevels without building
  them, and this repo's gate is subtractive so it is enforced the day it lands
  (nix/ci/flake-gate.nix). What it is not is *required*: ruleset 566717 carries
  no `required_status_checks` and every workflow is `workflow_dispatch` since
  #1088, so the check runs when somebody runs it. In index this directory was
  covered by a required context; that did not survive the move, and it is a
  mitigation rather than a restoration. `mcping.py` is still a command someone
  has to remember, and it is the one that would have caught the twelve hours of
  `active` above.

## What the live fleet is running, and how to check

**hyperion's root `flake.lock` is the record.** Not index's, not a pin, not this
file -- this file goes stale and the lock cannot. It names the platform revision
the guests are built from: index `69904c6e` as of 2026-07-30.

That claim is checkable rather than asserted. Realise each node's derivation
from the lock and compare against what the guests actually run:

```sh
# what the lock says the nodes should be
for a in hyperion-game-system hyperion-proxy-0-system \
         hyperion-proxy-1-system hyperion-proxy-2-system; do
  drv=$(nix eval --raw ".#packages.x86_64-linux.$a.drvPath")
  nix path-info --json "${drv}^out"
done

# what they are
for vm in hyperion-game hyperion-proxy-0 hyperion-proxy-1 hyperion-proxy-2; do
  ix shell "$vm" -- sh -c 'readlink -f /nix/var/nix/profiles/system'
done
```

`drv^out` and not `outPath`, for the reason in the section above: these are
content-addressed derivations and `outPath` is a placeholder.

### Rollback targets, a snapshot and therefore perishable

Read at **2026-07-30T06:15Z**, running hyperion `8b9a840` on index `69904c6e`:

    hyperion-game     system-14-link  skj499hs4324ksjfi3pccwg7fjs57iiw
    hyperion-proxy-0  system-8-link   6bcd07m258a8m34dspkmbznyr972s4f7
    hyperion-proxy-1  system-8-link   77knv9by7ggf5wvsnsy3kiqmv2qj6z03
    hyperion-proxy-2  system-5-link   ii74ikhydnpdzfy5slxfx412n98n6242

**This list is a snapshot and every apply invalidates it.** Do not trust it;
re-read it, which is one command:

```sh
for vm in hyperion-game hyperion-proxy-0 hyperion-proxy-1 hyperion-proxy-2; do
  ix shell "$vm" -- sh -c 'readlink /nix/var/nix/profiles/system'
done
```

Rolling one back, profile first so the pointer and the running system cannot
disagree:

```sh
ix shell <vm> -- sh -c 'nix-env --profile /nix/var/nix/profiles/system --rollback \
  && /nix/var/nix/profiles/system/bin/switch-to-configuration switch'
```

Measured: about one 2 s probe interval, because the closure is already on the
host.

### Index history holds the older definition, and it is not a recovery target

The fleet lived at `examples/minecraft/hyperion` in index until 2026-07-30, and
that copy is still in history:

```sh
git -C index checkout 7f602938fc30f184055384a2d212ec11d5b57df1 \
  -- examples/minecraft/hyperion
```

**Useful as history, misleading as a recovery target.** It pins index
`76e59e1a`, which is what the fleet ran *before* 2026-07-30T06:00Z and is not
current. Checking it out during an incident recovers the previous platform while
looking like it recovers the current one.

## Apply from a checkout of `main`, not from a branch

`nix build` and `ix apply` from a feature branch stamp that branch's commit into
`HYPERION_BUILD_REV`, and the game puts it on a boss bar in front of every
player. A branch commit is not reachable from `main`, so `git show <rev>` on a
fresh clone returns nothing and the natural conclusion is that the clone is
stale rather than that the stamp is meaningless.

It happened on 2026-07-30: the live server advertised `33e0d33` for ten minutes.
**Every other signal was green** -- apply exit 0, four `✓ ready`,
`NRestarts=0`, the `drv^out` identity check matching, the endpoint serving.
Nothing surfaces this except reading the deployed wrapper:

```sh
ix shell hyperion-game -- sh -c \
  'E=$(systemctl cat hyperion-game-server.service | grep -o "/nix/store/[^ ]*-smash-0.1.0-stamped");
   grep -o "HYPERION_BUILD_[A-Z]*=.[^\x27]*." "$E/bin/smash"'
```

Treat that as a standing post-apply check, and confirm the rev is one
`git merge-base --is-ancestor <rev> origin/main` accepts. ENG-11491 tracks
making the apply refuse an unreachable rev rather than relying on the habit.

## A dev machine in the fleet

`hyperion-dev` is a fifth node and the only one that serves nothing. It exists
so that the machine hyperion is built on is described in the same evaluation as
the machines hyperion runs on -- the same argument the header makes for the
fleet living in this repository at all, one step further back.

```sh
nix build .#hyperion-dev-system
ix apply .#hyperion-dev
ix shell hyperion-dev
```

Then, inside it, once:

```sh
cd /work/ix
git clone https://github.com/hyperion-mc/hyperion
```

`/work/ix` is the platform's own workspace directory
(`ix.profiles.base.shellWorkspace.directory`): it is pre-created by a tmpfiles
rule and login shells land in it, so it is where a checkout is findable by
somebody who did not make it. **Nothing clones for you, deliberately.** A clone
is state rather than configuration -- activation would have to pick a revision,
and every later apply would then either fight your working copy or ignore it,
which is a worse contract than having no opinion. Reading is public and needs
no credential; pushing needs yours, forwarded from your own machine, which is
the point.

The `nix build` line matters here for the same reason it does for the fleet
(see "Build the fleet once, not once per VM" above) and more so: this node's
closure carries a Rust toolchain the service nodes do not.

### It is not in the fleet's network segment

Every other node sets `ix.networking.groups = ["hyperion"]`, and that group is
the only thing keeping an unproxied client off the game server. This one
replaces it with `hyperion-dev`, so the box whose purpose is running code
nobody has reviewed yet has no route to the world. The build-and-push loop
needs none.

Groups are joined at VM create, like `ipv4`, so this is not a property a
re-apply can change on a VM that already exists: moving this node between
segments is `ix rm` and apply again.

### What the guest already has, and what it does not

Nearly all of a dev box is answered by the platform, so `nix/fleet/dev.nix`
adds a compiler and little else. Checkable in one command rather than believed:

```sh
nix eval --json .#nixosConfigurations.hyperion-dev.config --apply 'c: {
  workspace = c.ix.profiles.base.shellWorkspace.directory;
  git = c.programs.git.enable;
  features = c.nix.settings.experimental-features;
  substituters = c.nix.settings.substituters;
}'
```

which answers `/work/ix`, `true`, a feature list including `flakes` and
`ca-derivations`, and `cache.ix.dev` ahead of `cache.nixos.org`. That is what
this flake's `nixConfig` block asks of whatever evaluates it, already true
inside the guest, so the module restates none of it.

**The `ix` CLI is not in there and cannot be added from this repository.**
index packages no `ix` binary, and the CLI's repository is private while this
one is public -- an unauthenticated `GET /repos/indexable-inc/ix` answers 404
where `indexable-inc/index` answers 200 -- so a flake input naming it would
break evaluation for every outside contributor and for the gate that runs on
hosted runners. Type `ix` commands on your own machine against the VM. Anything
that needs the CLI *inside* the guest, such as an agent creating its own
sandbox, is out of reach until ENG-12081.

**There is no RAM, CPU or disk knob to set.** A fleet node takes `modules`,
`deployment`, `tags`, `groups`, `dependsOn`, `replicas` and `updateStrategy`,
and none of those is a machine size; `ix apply` and `ix new` have no sizing
flag either. Measured rather than assumed, from a node of this fleet:

```console
$ ix shell hyperion-game -- sh -c 'grep MemTotal /proc/meminfo; nproc'
MemTotal:       268435456 kB
64
```

So sizing is the platform's answer and not a limit worth designing around here.

### A temporary key, if something on the box needs the API

The loop above needs no ix credential at all: `ix` runs on your machine, and
git pushes over yours. A key is only wanted when something *on* the box calls
the ix API directly -- an agent that wants its own sandboxes, say, which today
means the HTTP API rather than the CLI (ENG-12081 again).

Mint it capped and narrow, store it, and attach it at create:

```sh
ix keys create hyperion-dev-agent --limit 25 --scope vm:read,create --rate-limit 60
ix secret set hyperion_dev_ix_key          # reads the value from a hidden prompt
ix new --name hyperion-dev --group hyperion-dev \
       --secret-env hyperion_dev_ix_key=IX_API_KEY --no-shell
ix apply .#hyperion-dev
```

The order is not a style choice. **`ix apply` cannot attach a secret**: it has
no `--secret-env`, and this fleet's `deployment.secrets` would be read by the
deprecated `ix-fleet` alone rather than by the created VM -- the split
`lib/image/fleet.nix` draws between workflow keys and create-identity keys, and
the same class of silent drop as ENG-10846. A secret therefore reaches a VM at
create, which means creating with `ix new` and converging with `ix apply`
afterwards. Revoke the key when the box goes: `ix keys revoke <id>` is terminal
and takes every key beneath it with it.

**None of those four commands has been run.** The node was added and evaluated,
no VM was created, and no key was minted; the sequence follows the CLI's own
contract (`ix apply` reuses a VM by name, `ix new` is the only path that
attaches a secret) rather than a run somebody watched. Expect to correct it the
first time, and correct it here.

**That key is broader than it reads, and this is the reason to keep it
temporary.** `--scope` parses `RESOURCE:ACTIONS` and nothing else, so there is
no syntax for naming which VMs it covers; and even a token that carried the ids
would not be narrowed by them, because the conversion from a token's scopes to
the permission set the server checks drops `resource_ids` on the floor
(`crates/ix/server/src/acl/auth.rs`, both `parse_db_scopes` and
`auth_context_from_validated`). A `vm:read` key therefore reads **every VM the
account owns**, this fleet's four included, not only the dev box it was minted
for. ENG-12039. Until that lands, the controls that actually bound the damage
are the spend cap, the rate limit, and revoking the key when you are done with
the machine.

### One node, no replicas, applied by name

`replicas` says the proxies are interchangeable. A dev machine is the opposite:
it holds somebody's working copy, so a second one is a second person's box and
not another copy of this one. Whoever wants that adds a node with their own
name on it.

Nothing stacks or scopes these per branch today. A `--stack` that gave each
branch its own copy of the fleet would change this section; there is no such
flag, so the fleet is applied by naming targets, and the dev node is applied on
its own when somebody wants it. Note the consequence of it being declared here:
a bare `ix apply .` converges every node this repository declares, which now
includes a dev box. Name your targets, which the commands above and at the top
of `nix/fleet/default.nix` already do.

Finally, "Apply from a checkout of `main`, not from a branch" above still
holds, and a dev box makes it easier to get wrong: the checkout in `/work/ix`
is usually on a branch, and applying the *game* fleet from there stamps an
unreachable commit onto a boss bar in front of every player.

## Checking the server answers: `mcping.py` moved

It is `nix/fleet/mcping.py` in this repository. **The old path,
`index/examples/minecraft/hyperion/mcping.py`, is gone** -- and it is the one in
everybody's shell history, so the first thing tried during an incident will fail
with `No such file or directory`.

```sh
python3 nix/fleet/mcping.py 15.204.111.75 25565
```

If a local checkout predates the move, read it out of the remote rather than
pulling mid-incident:

```sh
git show origin/main:nix/fleet/mcping.py | python3 - 15.204.111.75 25565
```

## Evaluating it without deploying

Every node's closure, from this directory, against the index checkout you are
editing:

```sh
nix eval --override-input index /path/to/index \
  .#nixosConfigurations \
  --apply 'cs: builtins.mapAttrs (n: c: c.config.system.build.toplevel.drvPath) cs'
```

11 seconds, and it forces all four systems, so a missing attribute, a bad option
or a port collision fails immediately. It builds nothing and deploys nothing: an
eval passing says the fleet is well formed, not that it works.

Two things about that command that cost time to find. Write the override as a
plain path, **not** `path:/path/to/index`: the `path:` fetcher copies the tree
into the store and `lib.fileset.gitTracked` then refuses it ("The argument is a
store path within a working tree of a Git repository"). And the plain path is a
git flakeref, so it reads `HEAD`, not your working tree -- commit before you
evaluate or you will confidently evaluate the previous version.

Holding the index input constant also makes the eval a diff tool. Evaluating
this directory's current spec against the index revision that predates it
reproduces the previous fleet's derivation paths exactly:

    hyperion-game     k4nxy1l152lq4vq8wfv7q5d926nxqzl8   unchanged
    hyperion-proxy-0  aragzfail81z0fdz38dzgzm126jqvrjj   unchanged
    hyperion-proxy-1  s1k599cw6blsh9927f3cv8l0bh7f7y4v   unchanged
    hyperion-proxy-2  8ir5pdfimwh89c375cq8hml44yxqpk7n   new

So folding the two copy-pasted proxy nodes into `replicas: 3` and deleting the
resolved DNS workaround changed no existing node's system at all -- re-applying
switches and restarts nothing, and the only new closure is the third proxy.
Worth doing before any apply that claims to be a no-op.

Compare `drvPath`, and only `drvPath`. Everything in this fleet is a
content-addressed derivation, which breaks the two comparisons you would reach
for first:

- **`outPath` is a placeholder, not a path.** `nix eval .#...smash.outPath`
  returns `/0rr9ly5rvixfvxzdrphms8722jikwmqq5m12rwvhk62d517ql1bk`. It is not a
  store path, it does not exist, and it is the same shape for every CA
  derivation, so two different builds compare equal.
- **`nix path-info --deriver` returns the *resolved* drv, not the evaluated
  one.** A CA build rewrites its input placeholders into real paths and records
  that resolved derivation as the deriver. So the deriver of a running binary
  legitimately differs from the `drvPath` your eval just produced, and the
  difference is not drift.

Both failures look exactly like a stale pin, which is the problem: they are
loudest when nothing is wrong. To tie a running artifact to a revision, resolve
the realisation of the evaluated derivation and compare store paths:

    $ nix eval --raw --impure --expr '(builtins.getFlake
        "path:/path/to/hyperion").packages.x86_64-linux.smash.drvPath'
    /nix/store/3y6c24fgz20s2s4iwjiya010mz71ifyk-smash-0.1.0.drv

    $ nix path-info --json '/nix/store/3y6c24fgz20s2s4iwjiya010mz71ifyk-smash-0.1.0.drv^out'
    {"/nix/store/nvm38n41rsipjak0gbzjxvfxz8ddm66n-smash-0.1.0": ...}

    $ ix shell hyperion-game -- sh -c 'systemctl cat hyperion-game-server.service' | grep ExecStart
    ExecStart=/nix/store/nvm38n41rsipjak0gbzjxvfxz8ddm66n-smash-0.1.0/bin/smash ...

The `drv^out` suffix is the whole trick: it asks the realisation, which is the
only thing that knows which output a CA derivation actually produced.

The same trick checks that the replicas really are interchangeable rather than
merely evaluating. `nix derivation show` on two of them differs in 2 of 17 input
derivations, `etc` and `activate`, and in exactly two attributes, `name` and the
`buildCommand` that embeds the `etc` path. `system-path` and every package under
it are the same store path. The only thing separating one proxy from the next is
its hostname, which is what "interchangeable" has to mean to be worth saying.

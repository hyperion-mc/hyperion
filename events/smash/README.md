# smash

Mineplex's Super Smash Mobs for hyperion.

Damage does not kill you. Damage lowers your health bar, a low health bar makes
every subsequent hit launch you further, and eventually a hit launches you off
the map. Everything else in the game is downstream of that one formula, which is
transcribed from Mineplex's own source and lives in
[`src/module/knockback.rs`](src/module/knockback.rs).

## Layout

Every subsystem and every kit is a flecs [`Module`]. `SmashModule` is nothing
but an import list.

```
src/
  server.rs          the seam to the host Minecraft server: nine write methods
  server/mock.rs     a recording test double, so the game runs headless
  flecs_ext.rs       fixes to the flecs_ecs API, carried until they land upstream
  map.rs             the map file format, its parser, and the list of maps
  terrain.rs         map files into the world's blocks; the region layout
  module/
    player.rs        mirrored position/velocity/facing, health, energy, jumps
    damage.rs        armour, the Damaged event, kill attribution
    knockback.rs     the formula, as pure functions over a tunable singleton
    ability.rs       abilities as entities; one dispatcher for the whole game
    kit.rs           kits as prefabs, and the builder a kit file uses
    projectile.rs    arrows, hooks, thrown blocks: one component set, one system
    arena.rs         the live arena singleton, and the death checks
    lives.rs         four lives, death, respawn, elimination
    lobby.rs         hub to countdown to match to results, as a pure function
    scoreboard.rs    the sidebar, and spectating on elimination
    kits.rs          the list of kits, which is only ever imports
    kits/            skeleton, iron_golem, enderman, slime
maps/              one file per arena, plus the waiting lobby
```

## Adding a kit

One file and one line. No match statement, no enum, no dispatch table.

```rust
#[derive(Component)]
pub struct Wolf;

impl Module for Wolf {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Wolf");

        kit::define(world, "Wolf", KitStats {
            melee_damage: 5.0,
            armor: 9.0,
            knockback_taken: 1.6,
            ..KitStats::default()
        })
        .ability(AbilitySpec {
            name: "Wolf Strike",
            item: "minecraft:iron_shovel",
            slot: 1,
            cooldown: 6.0,
            activate: wolf_strike,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}
```

then `world.import::<Wolf>()`. `tests/modularity.rs` proves the claim by
defining a kit from outside the crate and asserting no file under `src/`
mentions it.

## Adding a map

One file under [`maps/`](maps) and one line in `map::ARENAS`. No Rust
otherwise: a map is data, the way Mineplex's were, so a wrong island is fixed by
editing a text file rather than by knowing Rust.

A map file is one directive per line, `#` to end of line for comments. Every
coordinate is local to the map, and every range is inclusive, which is how a
builder thinks about a platform ("y 63 to 66"):

```
name Sky Fortress            # runs to the end of the line; no quotes needed
author whoever built it

kill_y 20                    # below this Y a player is dead

box      -6 65 -2  -4 67 2   minecraft:stone      # two inclusive corners
cylinder  0 64  0  16 1      minecraft:grass_block  # centre, radius, height up
sphere   -8 71 -8  3         minecraft:oak_leaves   # centre, radius
cone      0 61  0  15 12     minecraft:stone        # centre, radius, depth down

spawn   -10 65 -10           # where the scatter and respawns put players
crystal   0 68 0             # where a Smash Crystal may land
```

Brushes are stamped in the order they appear and a later one overwrites an
earlier one, which is how the lobby carves the inside out of its glass ring with
a cylinder of air. `cone` tapers downwards from its centre to a point, which is
the underside every floating island has.

`parse` refuses a file rather than degrading it: an unknown directive, a block
id without its `minecraft:` prefix, no `name`, no `spawn`, no `kill_y`, or a
kill plane at or above the lowest spawn. That last one is not hypothetical --
it is the shape of the bug that made the old downloaded world unplayable, where
everyone died on the tick they were placed.

Two more checks run over the shipped maps and will fail the build or the tests
rather than the match:

- every spawn point has a solid block under it. hyperion reads `ceil(y) - 1` to
  decide a player is standing, so a spawn one block too high leaves them
  airborne forever and every grounded ability silently refuses to fire.
- the kill plane is below every block the map places, so nobody dies standing on
  real terrain.

[`tests/maps.rs`](tests/maps.rs) has both, plus one test per rejection above.

### Where the maps live in the world

There is one world and no world switching, because hyperion serves exactly one
set of chunks. Each map gets its own slot along +X, `terrain::REGION_STRIDE`
apart, far enough that no view distance reaches from one to the next. The lobby
is region 0 and the arenas follow in `map::ARENAS` order. Choosing a map for the
next match is therefore a teleport, not a world load, and the x coordinate of
where a player lands says which map they are on.

`MapRotation` picks the next one when a match ends rather than when the next one
starts, because `Lobby::scatter` reads the `Arena` singleton on the way into
`Preparing` and would otherwise use the previous map. The order is the order of
`map::ARENAS`, so it is deterministic and a test can predict it.

### Checking a map on a real client

`nix run .#smash-map-e2e` brings the stack up and drives
[`tools/smash-map-check.py`](../../tools/smash-map-check.py) at it: a protocol
776 client that decodes the world off the wire and compares it, block for block,
against the files in `maps/`. It then walks a player off the edge, hovers five
blocks above the declared kill plane long enough to prove that height is
survivable, and drops through to prove it is not.

Reading the world takes two packets and not one. `terrain.rs` builds on
`Blocks::empty`, so every column is encoded as air before a single block is
stamped into it and that encoding is never rebuilt; a joining player gets the
empty chunk followed by `section_blocks_update` carrying every change since the
column loaded. A client that decodes only `level_chunk_with_light` sees a world
with no floor in it.

## Running it

```sh
nix run .#certs   # once: the game server and the proxy authenticate over mTLS
nix run .#smash   # game server with a proxy inside it, on localhost:25565
```

`nix run .#smash-dev` is the same thing split into the deployed shape -- game
server, separate proxy -- under a rebuild-and-restart watcher, the way
`nix run .#dev` does it for bedwars.

Once in the world: `/kits` lists what is available, `/kit skeleton` picks one.
Names are matched with case and spaces discarded, so `/kit irongolem` is
`Iron Golem`. Right-click a hotbar slot to use the ability bound to it; left
click to swing.

To check the server is genuinely joinable rather than merely accepting
connections, drive [`tools/smash-client.py`](../../tools/smash-client.py) at it.
It is a scripted 1.20.1 client that distinguishes "authenticated" from "in the
world" -- the distinction the proxy's connection count cannot make, and the one
that separates a working server from a client stuck on *Joining world...*
forever:

```sh
python3 tools/smash-client.py --port 25565 --name Alpha --command "kit skeleton"
```

## The host half

`SmashModule` is the game and depends on nothing but `flecs_ecs` and `glam`.
`SmashHost` is the game plus hyperion, and everything hyperion-shaped is in
four files:

```
src/
  adapter.rs   implements Server against hyperion; the only file importing both
  mirror.rs    hyperion position/facing/ground state onto the game's mirrors
  input.rs     packet events into ability activations, damage and kit hotbars
  command.rs   /kit and /kits
  main.rs      argument parsing and the entry point
```

Writes cross the seam as a queue drained once per tick rather than as immediate
world edits, because `Server` is called from inside observers -- ability
activation, the damage pipeline, the lobby -- where taking a second mutable
borrow of a component a running query holds is a runtime abort. The cost is one
tick of latency on knockback.

Nothing under `src/module/` changed to make any of this work, which was the
design's own test of whether the seam was in the right place.

## Building

`flecs_ecs 0.2.2` declares an MSRV of 1.88, which the repository's pinned
`nightly-2025-05-05` satisfies, so the ordinary gates cover this crate:

```sh
nix run .#test
nix run .#lint
```

## Documents

- [`docs/smash-design.md`](../../docs/smash-design.md) — the mechanics, with
  sources, and what is verified versus inferred; the architecture; and the
  file-by-file wiring to hyperion.
- [`docs/flecs-rust-api-notes.md`](../../docs/flecs-rust-api-notes.md) — every
  place the flecs Rust API fought back, what was changed, and what is going
  upstream.

[`Module`]: https://docs.rs/flecs_ecs/0.2.2/flecs_ecs/addons/module/trait.Module.html

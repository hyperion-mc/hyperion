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
  module/
    player.rs        mirrored position/velocity/facing, health, energy, jumps
    damage.rs        armour, the Damaged event, kill attribution
    knockback.rs     the formula, as pure functions over a tunable singleton
    ability.rs       abilities as entities; one dispatcher for the whole game
    kit.rs           kits as prefabs, and the builder a kit file uses
    projectile.rs    arrows, hooks, thrown blocks: one component set, one system
    arena.rs         platforms, the kill plane
    lives.rs         four lives, death, respawn, elimination
    lobby.rs         hub to countdown to match to results, as a pure function
    scoreboard.rs    the sidebar, and spectating on elimination
    kits.rs          the list of kits, which is only ever imports
    kits/            skeleton, iron_golem, enderman, slime
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

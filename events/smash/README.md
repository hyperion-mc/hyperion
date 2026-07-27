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

## Building

`flecs_ecs 0.2.2` declares an MSRV of 1.88, above the repository's current pin.
Use the toolchain the in-flight flecs migration already pins:

```sh
RUSTUP_TOOLCHAIN=nightly-2025-05-05 cargo test -p smash
RUSTUP_TOOLCHAIN=nightly-2025-05-05 cargo clippy -p smash --all-targets --all-features -- -D warnings
```

This crate does not depend on `hyperion` yet, because hyperion is mid-migration
from `bevy_ecs` back to `flecs_ecs`. Everything the host would provide is behind
the `Server` trait in [`src/server.rs`](src/server.rs), with a recording double
in [`src/server/mock.rs`](src/server/mock.rs), so the game logic runs and is
tested today.

## Documents

- [`docs/smash-design.md`](../../docs/smash-design.md) — the mechanics, with
  sources, and what is verified versus inferred; the architecture; and the
  file-by-file wiring to hyperion.
- [`docs/flecs-rust-api-notes.md`](../../docs/flecs-rust-api-notes.md) — every
  place the flecs Rust API fought back, what was changed, and what is going
  upstream.

[`Module`]: https://docs.rs/flecs_ecs/0.2.2/flecs_ecs/addons/module/trait.Module.html

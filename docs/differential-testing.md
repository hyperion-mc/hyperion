# Differential testing against the real server

Hyperion's simulation is checked tick by tick against Mojang's, by recording
what the real server does and replaying it here. `cargo test -p hyperion --test
differential` runs the comparison with no Java, no network and no server: the
expected numbers are committed JSON. `nix flake check` re-records them from the
pinned jar and fails if they have moved, so a version bump cannot leave the
golden data quietly wrong.

The first thing it found is that hyperion's projectile physics was not vanilla's
in three separate ways, and later that the heading it sent to the client was a
fourth. See "What this has already caught" at the bottom.

## Adding a case is adding a file

There is no per-scenario code. Drop a JSON file in
`crates/hyperion/tests/differential/scenarios/`, record it, and the test picks
it up:

```sh
nix run .#record-differential-traces
cargo test -p hyperion --test differential
```

The file name must match the `name` inside it, and every key is required, since
a scenario with a misspelled key would otherwise be silently compared against
the wrong recording.

```json
{
  "name": "arrow-level-shot",
  "description": "A fully drawn bow fired dead level along +Z.",
  "ticks": 60,
  "seed": 4242,
  "blocks": [
    { "position": [0, 120, 10], "state": "minecraft:stone" }
  ],
  "entities": [
    {
      "id": "arrow",
      "type": "minecraft:arrow",
      "position": [0.5, 128.0, 0.5],
      "launch": { "yaw": 0.0, "pitch": 0.0, "power": 3.0 }
    }
  ],
  "compare": {
    "position": 5.0e-4,
    "velocity": 1.0e-5,
    "rotation": 2.0e-1
  }
}
```

| Field | Meaning |
| --- | --- |
| `name` | Matches the file name and the trace file name. |
| `description` | For a reader. Say what a player would be doing. |
| `ticks` | Server ticks to record. The trace carries `ticks + 1` samples, since sample 0 is the state before the first tick. |
| `seed` | The world seed the committed trace was recorded under. |
| `entities[].id` | Names this entity in the trace and in a failure message. |
| `entities[].type` | A protocol entity type. It must appear in `hyperion::simulation::projectile_motion::SIMULATED`, or there is nothing here to compare. |
| `entities[].position` | Where it starts. |
| `blocks` | Optional. Terrain to put in the world before anything is fired. |
| `blocks[].position` | Integer block coordinates. |
| `blocks[].state` | A block name and nothing else: `minecraft:stone`, not `minecraft:stone_slab[type=top]`. Both sides place the block's default state. |
| `compare.position` | Tolerance in blocks. |
| `compare.velocity` | Tolerance in blocks per tick. |
| `compare.rotation` | Tolerance in degrees, for the arrow's client-facing yaw and pitch. |

Exactly one impulse per entity, and it is applied by vanilla, not by the
scenario:

- `"launch": { "yaw", "pitch", "power" }` runs `Projectile.shootFromRotation`
  at zero inaccuracy. A fully drawn bow is `power: 3.0`; a thrown snowball is
  `1.5`.
- `"motion": [x, y, z]` sets the delta movement directly.
- `"knockback": { "power", "fromX", "fromZ", "damage", "onGround" }` runs
  `LivingEntity.knockback`, for entities that have one.

### Terrain is opt-in, per scenario

A scenario with no `blocks` runs in an empty world on both sides and its trace
is unchanged by any of this: the recorder places nothing and the replay stamps
nothing. There is no global "load a flat world" switch to get wrong, and adding
the terrain scenarios changed the four sky scenarios' recorded numbers not at
all -- the only difference in those files is the two impact fields and the
header index described below.

A scenario that *does* name blocks gets them in both places before any entity
exists. `VanillaTrace.placeBlocks` calls `setBlock` with `Block.UPDATE_CLIENTS`
rather than `UPDATE_ALL`, because a neighbour update runs block logic and block
logic is where a recording would start consuming randomness. On the hyperion
side `stamp_terrain` loads each containing chunk first -- `HyperionCore`
installs `Blocks::empty`, and `set_block` on an unloaded chunk quietly places
nothing -- and clears them again afterwards, because every scenario shares one
world where vanilla records each in a fresh level.

Default states only. That means the two registries' defaults have to agree, and
that is checked rather than trusted: a slab that came out `top` on one side and
`bottom` on the other moves the arrow's resting height by half a block, four
orders of magnitude outside any tolerance here.

### The impact state, and the field index that rides with it

An arrow's trace also carries `inGround` and `shakeTime`, and they are compared
exactly, with no tolerance -- a flag and a countdown have no notion of "close".
They are the reason a terrain scenario can assert anything at all: a resting
position on its own cannot tell "stopped by the wall" from "still flying and
happening to be there this tick". A snowball's trace carries neither, because
`ThrowableProjectile` has no such state.

`inGround` is read from the *synched* data rather than from `isInGround()`,
which is `protected` -- so it is the value a client is actually sent, which is
what `metadata::arrow::InGround` mirrors. Getting the accessor means reflection,
and since the reflection is happening anyway the trace header records
`inGroundFieldIndex` as well. That number is the one thing in
`crates/hyperion/src/simulation/metadata/` nothing else can check: a field index
never appears on the wire, so no packet capture recovers it, and getting it
wrong neither fails to compile nor fails to send -- it writes a boolean into
whichever field Mojang moved into slot 10 and the arrow quietly does something
else on the client. The replay asserts it against `InGround::INDEX` before it
compares a single tick. ENG-12106 is the general version of this for every other
hand-transcribed index.

## What is deterministic, and how that is known rather than assumed

An uncontrolled random process compared against itself passes and means
nothing, so this is the part worth reading.

Nothing on the measured path consumes randomness. `AbstractArrow.tick`,
`ThrowableProjectile.tick`, `Projectile.shoot` at zero inaccuracy and
`LivingEntity.knockback` for a non-degenerate direction are branch-free
arithmetic on doubles. Mob spawning, daylight, weather and random block ticks
are turned off, and the world is the flat preset, which has no structures and
no terrain variation.

That claim is checked rather than believed. **The recorder replays every
scenario under three different world seeds and refuses to emit a trace unless
all three agree byte for byte.** Vanilla derives the level's random source from
the world seed, so a scenario that reaches it produces three different
recordings and fails the build with the diff. This is why there is no Java
agent seeding `RandomSource`: not seeding it and proving independence is a
stronger statement than seeding it and asserting one.

### Across platforms

`Mth`'s sine table is built with `Math.sin`, which the JLS allows to be off by
an ulp and does not require to agree between implementations, so a recording
made on one machine could in principle differ from one made on another and the
drift check would fail for whoever ran it second. It does not: recording the
committed scenarios on `x86_64-linux` and on `aarch64-darwin` produces
byte-identical files. Worth re-checking on a JDK bump rather than assuming,
since it is a property of the runtime and not of anything in this repository.

### What this therefore does not prove

- **The bow's spread.** `Projectile.shoot` adds a random triangular term scaled
  by inaccuracy, and a real bow shot passes inaccuracy 1.0. Scenarios pass 0.0,
  so the spread distribution is untested. What is tested is the flight, from
  whatever state vanilla started it in.
- **Entity collision, and despawn.** `removed` is recorded and nothing asserts
  on it, and no scenario puts a second entity in an arrow's path. Block
  collision *is* covered now -- see the three terrain scenarios -- but what an
  arrow does when it meets a player is not.
- **Water, lava, portals, bubble columns, levitation.** All change the
  integration and none appear in a flat world's sky.
- **Player movement, and so knockback as Super Smash Mobs uses it.** Two
  separate reasons, and both are worth saying plainly. First, smash's knockback
  formula is Mineplex's, deliberately not vanilla's, so vanilla has no opinion
  to compare against; `docs/smash-design.md` is the reference for that half.
  Second, the part vanilla *does* own -- the arc a launched player follows
  afterwards -- lives in `MovementTracking::server_velocity`, which only runs
  for an entity with a live `ConnectionId`, so it cannot be driven from a
  headless test today. The scenario format already carries a `knockback`
  impulse for when it can.

## Why the tolerance is what it is

It is a bound, not a number tuned until the test went green.

Vanilla stores positions and velocities as `double`. Hyperion stores them as
`f32`, for cache locality, and that is the only remaining source of
disagreement once the physics matches. Each tick rounds each component to `f32`
at least once, contributing at most half an ulp of that component's magnitude,
so after `n` ticks the two cannot differ by more than `n * ulp(max |value|) / 2`
even if every rounding goes the same way.

For `arrow-level-shot`: 60 ticks, coordinates reaching 136 blocks, where an
`f32` ulp is `2^-16`. That is `60 * 2^-17`, about `4.6e-4`, and the committed
tolerance is `5e-4`. Velocity peaks at 3.0 blocks per tick, ulp `2^-22`, giving
`7.2e-6` against a committed `1e-5`.

The test prints the worst delta it actually saw next to the tolerance, so the
headroom is visible on every green run rather than only in this document:

```
ok: arrow-level-shot (60 ticks); worst position delta 4.964110533478561e-5 of 5e-4, velocity 1.4199340703235919e-6 of 1e-5
ok: snowball-throw (40 ticks); worst position delta 1.4786633116159464e-5 of 4e-4, velocity 5.365354273090261e-7 of 3e-6
```

Both sit about an order of magnitude inside the bound, which says the rounding
errors are not accumulating in one direction.

The terrain scenarios are twenty ticks and reach 128 blocks (121 for the slab),
so the same arithmetic gives `20 * 2^-17 = 1.5e-4` and `20 * 2^-18 = 7.6e-5`;
the committed tolerances are `2e-4` and `1e-4`. What they actually measure is
smaller again, and this is the number worth reading, because the question going
in was whether a swept clip against block shapes would open a bigger gap than
free flight does:

```
ok: arrow-into-floor (20 ticks);   worst position delta 3.386e-6 of 2e-4
ok: arrow-into-wall (20 ticks);    worst position delta 5.615e-6 of 2e-4
ok: arrow-grazing-slab (20 ticks); worst position delta 8.309e-6 of 1e-4
```

It does not. Those are *tighter* than the sixty-tick sky shots (3.4e-5 to
5.0e-5), for the ordinary reason that they run a third as long. The clip itself
contributes nothing measurable: `geometry::sweep::first_block_hit` computes in
`f64` internally, so the only rounding on the impact path is the `f32` the
segment's endpoints arrive as -- the same single source of disagreement free
flight has. **The residual gap is hyperion's `f32` positions, and nothing else.**

**This tolerance is larger than the wire can express, and that is a real
finding rather than a detail.** Entity position deltas go on the wire in units
of 1/4096 of a block, about `2.4e-4`, so hyperion's `f32` storage is on the edge
of being visible to a client on a long flight. Narrowing this would mean
storing projectile state as `f64`, which is a change to `Position` and
`Velocity` across the whole crate, not to this test.

## How the recording works

Three pieces, because the fast inner loop and the source of truth should not be
the same thing.

1. **`nix/java/VanillaTrace.java`** is a `MinecraftServer` subclass that loads
   a flat world, spawns the scenario's entities, ticks, and writes one JSON
   sample per tick.
2. **`crates/hyperion/tests/differential/traces/`** holds the committed
   recordings, so the everyday test is a plain Rust test.
3. **`checks.differential-traces`** re-records and diffs, so the committed copy
   stays honest.

### Why a server subclass and not the published server

The dedicated server binds a listening socket during startup, and a nix build
sandbox denies `bind`, so it cannot run in a derivation at all:

```
Exception in thread "main" java.net.SocketException: Operation not permitted
        at java.base/sun.nio.ch.Net.bind0(Native Method)
```

Mojang hit the same problem running their own game tests in CI and answered it
with `GameTestServer`, a `MinecraftServer` that never opens a port.
`VanillaTrace` is that recipe with the test runner replaced by a recorder. It
also buys exact control of the sampling phase, which a datapack scraping the
console log would not have.

### The sampling phase

Sample 0 is the state immediately after the entity is added to the level and
before any tick. Sample `k` is the state after `k` calls to
`MinecraftServer.tickServer`, which is what advances the level. On the hyperion
side the same sample is read after `world.progress()`. Both are "end of tick".

### Chunks

An entity only ticks inside a chunk that has climbed to entity ticking, and
nothing here loads chunks the way a nearby player would. The recorder claims
every chunk within `ticks * initial_speed` of the start -- a bound, not a
guess, since no vanilla drag term increases speed -- and then waits for all of
them to reach entity ticking *before* spawning anything. Skipping that wait
produced a trace of an arrow that sat still for 35 ticks, started moving, and
stopped again on leaving the loaded region, with nothing in the file to say so.

## What this has already caught

Hyperion integrated every projectile as

```rust
velocity *= 0.997_525;   // "Drag (0.99 / 20.0)"
velocity.y -= 0.05;
position += velocity;
```

Three things are wrong with that, and the arrow scenario fails on all three:

1. **The drag constant.** The comment reads `1.0 - (0.99 / 20.0) * 0.05`, which
   treats vanilla's 0.99 as a per-second rate. It is per tick.
2. **The order.** `AbstractArrow.tick` moves *first* and decays afterwards, so
   an arrow travels its full launch speed on its first tick. Hyperion decayed
   first, losing 0.03 blocks on tick one of a level shot and never recovering.
3. **One integrator for everything.** `ThrowableProjectile.tick` is a different
   shape from `AbstractArrow.tick`, not just different constants: gravity, then
   drag, then move. A snowball is already slowed and falling before it moves at
   all.

`crates/hyperion/src/simulation/projectile_motion.rs` now carries both shapes as
data, and this test is what keeps them honest.

### The heading

The fourth finding is the one a player sees. An arrow's arc was right, but the
orientation the server sent was wrong twice over.

A projectile entity does not store the shooter's look angles. Vanilla derives
its yaw and pitch from its velocity every tick, in `AbstractArrow.tick` and
`Projectile.updateRotation`, as `yaw = atan2(dx, dz)` and
`pitch = atan2(dy, horizontalDistance)`. That is the sign-flip of the look
convention a shooter's own yaw uses: an arrow loosed due west stores yaw -90
where the player who fired it reads +90. Hyperion set the arrow's yaw and pitch
to the shooter's own, so every arrow rendered mirrored across its line of
flight. `arrow-crosswind-shot` catches this at tick 0, +90 against a recorded
-90.

And vanilla re-aims the arrow off its velocity every tick, easing 20% of the
way each time (`lerpRotation`), so the arrow noses over as it falls. Hyperion
never updated the orientation after launch, so it stayed frozen at its loosed
angle for the whole flight. `arrow-arced-shot` catches this: vanilla's pitch
climbs from +20 to -43 over sixty ticks while a frozen arrow sits at +20.

The two integrators disagree about *when* the arrow is aimed, the same way they
disagree about when it moves. `AbstractArrow.tick` aims from the velocity it
entered the tick with, before the move and decay; `ThrowableProjectile.tick`
applies gravity and drag first and aims from the result. `look_angles` and the
per-order aim in `update_projectile_positions` carry both, and the rotation
column of every trace keeps them honest. Hyperion aims with `f32::atan2` where
vanilla uses `Mth.atan2`, a table approximation; the two agree to under a
thousandth of a degree across every committed scenario, which is why the
rotation tolerance is a fifth of a degree rather than zero.

### The heading of an arrow that stops

The terrain scenarios found this on their first run, which is the reason to
write them.

`AbstractArrow.tick` aims the arrow at lines 212-215, from the velocity it
entered the tick with, and **before** the clip at line 218. So an arrow that
meets a wall this tick still turns to face the way it was going, and then holds
that heading for as long as it stays embedded, because the in-ground branch
returns at line 199 without reaching the rotation again.

Hyperion did the aiming inside the *miss* branch of
`update_projectile_positions`, so an arrow that landed kept the heading it had
one tick earlier -- exactly one `lerpRotation` step behind, forever:

```
arrow-into-wall: arrow pitch diverges at tick 4
  vanilla:  -1.0176632
  hyperion: -0.5419483780860901
  delta:    4.757e-1 (tolerance 2e-1)
```

Nothing else could have found it. It is invisible in flight, where the next tick
corrects it; it is invisible to every sky scenario, because they never stop; and
it is invisible to the bow e2e checks, which read velocity rather than
orientation. The fix is one line moved above the clip, and the comment there
names the vanilla lines rather than the symptom.

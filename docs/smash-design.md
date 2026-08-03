# Super Smash Mobs

A reimplementation of **Mineplex's** Super Smash Mobs — not Hypixel's Smash
Heroes — for the hyperion engine, in `events/smash/`.

This document records what the original actually did and where that was
established from, then the architecture built on it, then what is missing.
Every number is labelled with its provenance. Where a number could not be
sourced it says so rather than guessing.

## Provenance, up front

Three kinds of source, in descending order of authority.

**`[SOURCE]` — Mineplex's own Java.** Mineplex's server code leaked and was
DMCA'd in May 2023 ([GitHub's notice][dmca]); a working mirror survives at
[`timing1337/mineplex-reborn`][mirror]. It is a ~2018, Minecraft 1.8.8 snapshot.
The files that matter here are `SuperSmash.java`, `PerkSmashStats.java`,
`DamageManager.java`, `UtilAction.java` and `PerkDoubleJump.java`.

**`[SHEET]` — the balance spreadsheet.** `SuperSmash.java` registers
`new PerkSpreadsheetModule("SMASH_KITS")`, so every kit stat and every ability
number is loaded from a Google Sheet at runtime and *is not in the code*. The
mirror carries a dump of it at `update/files/SMASH_KITS.json`. This is why the
Java documents the mechanism perfectly and the numbers not at all.

**`[WIKI]` — the [Mineplex Fandom wiki][wiki] and the [official site][site].**
Used where nothing better exists, and where it agrees with the sheet it is
quoted as corroboration. Where it disagrees, the sheet wins and the
disagreement is noted.

**`[INFERRED]` — my choice.** Called out every time.

A recurring caveat: the relaunched Mineplex (2024 onwards) is a different,
closed codebase. Its [2026 changelog][changelog] shows the model is
substantially the same but retuned. Where behaviour has demonstrably changed the
newer value is used and cited.

[dmca]: https://github.com/github/dmca/blob/master/2023/05/2023-05-23-mineplex.md
[mirror]: https://github.com/timing1337/mineplex-reborn
[wiki]: https://mineplex.fandom.com/wiki/Super_Smash_Mobs
[site]: https://mineplex.com/games/super-smash-mobs/
[changelog]: https://mineplex.com/threads/super-smash-mobs-update.19926/
[guide]: https://mineplex.com/threads/super-smash-mobs-unofficial-guide.20472/

---

## Damage does not kill you; being light does

This is the whole game, and everything else is downstream of it.

In Super Smash Mobs you have an ordinary twenty-point Minecraft health bar that
regenerates on its own. Losing health does not kill you. What losing health does
is make every subsequent hit launch you further, until one launches you off the
edge of the map. It is Smash Bros' rising damage percentage, restated as a
falling health bar.

`[SOURCE]` `SuperSmash.java:533` is the line:

```java
event.AddKnockback("Smash Knockback", 1 + 0.1 * (player.getMaxHealth() - player.getHealth()));
```

`[SOURCE]` `DamageManager.java:518-560` is the rest of it:

```java
double knockback = Math.log10(Math.max(event.GetDamage(), 2));
for (double cur : event.GetKnockback().values())
    knockback *= cur;                                    // everything is multiplicative

Vector trajectory = UtilAlg.getTrajectory2d(origin, damagee.getLocation());
trajectory.multiply(0.6 * knockback);
double vel = 0.2 + trajectory.length() * 0.8;

UtilAction.velocity(damagee, trajectory, vel,
        false, 0, Math.abs(0.2 * knockback), 0.4 + (0.04 * knockback), true);
```

and `UtilAction.velocity` finishes it: normalise, scale to `vel`, add the
vertical, clamp the vertical to `yMax`, then add another `0.2` if the victim is
standing on something.

Consolidated, in blocks per tick:

```
K          = log10(max(damage, 2))
             × (1 + 0.1 × (maxHealth − health))    ← the health term
             × kitKnockbackTaken                    ← 1.25 for Skeleton, 1.75 for Slime
             × Π(ability multipliers)               ← 2.5 for Bone Explosion

horizontal = 0.2 + 0.8 × (0.6 × K)   =  0.2 + 0.48 × K
vertical   = min(0.2 × K, 0.4 + 0.04 × K)  + (grounded ? 0.2 : 0)
```

Three consequences worth stating separately, because they are what the game
*feels* like:

**A near-dead player is launched about 2.4× as far by the same hit.** The health
term runs from 1.0 at full health to 2.9 at one health.

**Hard hits are flatter, not higher.** The vertical cap `0.4 + 0.04·K` grows
twelve times more slowly than the uncapped `0.2·K` it caps, so past `K = 2.5`
everything extra goes sideways. This is why Super Smash Mobs kills you off an
edge and not into the sky. `tests/knockback.rs::launch_angle_peaks_where_the_vertical_cap_starts_binding`
pins both halves of that curve.

**Multipliers compose multiplicatively.** A 150% kit hit by a 2.5× ability takes
3.75×, not 2.65×. This is why one Bone Explosion on a chipped Slime is lethal
and why every per-kit passive that touches knockback is expressed as a factor.

`[SOURCE]` Fall damage is cancelled outright, so a purely vertical launch is
survivable. Vanilla knockback is replaced wholesale, so the vanilla sprint bonus
and the Knockback enchantment are both irrelevant.

### How this differs from vanilla

`[WIKI]` Vanilla Minecraft knockback is a fixed 0.4 horizontal and 0.4 vertical,
plus 0.5 per level of Knockback, doubled while sprinting, and it never looks at
the victim at all. That last property is the one Super Smash Mobs had to
discard, and `tests/knockback.rs::vanilla_ignores_the_victim_and_smash_does_not`
holds the two side by side.

### Implementation

`events/smash/src/module/knockback.rs`. `strength` and `resolve` are pure
functions over plain numbers with no world access, so the table tests drive them
directly. The constants live in a `KnockbackModel` singleton rather than in the
code, which is how Mineplex had it too — theirs was in a spreadsheet.

`resolve` keeps Mineplex's redundant round trip through a vector length
(`0.6 · K`, then `0.2 + 0.8 · length`) rather than collapsing it to
`0.2 + 0.48 · K`, so the constants stay recognisable against the original.

---

## Armour is vanilla, and hunger deliberately ignores it

`[SOURCE]` + `[WIKI]` Reduction is `armourPoints × 4%`, capped at 80% — plain
vanilla 1.8. The wiki's own pairings confirm the arithmetic: Skeleton's 12
points are listed at 48%, Iron Golem's 16 at 64%.

`[SOURCE]` A trap here: `PerkSmashStats._armor` is read from the sheet and shown
in the kit lore but **never applied**. Real mitigation comes from the equipped
armour set. The sheet's "Armor" column is vanilla armour *icons*, which is points
÷ 2, and that is the entire reason the wiki appears to contradict itself on
armour values.

`[SOURCE]` + `[changelog]` Hunger drains half a shank every 7.75 seconds for
most kits (7 for Skeleton, Zombie, Slime, Sir. Sheep, Wither Skeleton, Snowman,
Skeletal Horse and Villager; 6.25 for Spider, Wolf, Magma Cube and Chicken), and
at zero it deals half a heart per second as **true damage**. It ignores armour
on purpose: the old code ran it through armour, which let high-armour kits
outlast everyone in a stalled game, and the relaunch fixed it. There is no
sudden death in Super Smash Mobs; the starve timer is what it has instead.

`[WIKI]` The bar is not a one-way clock: "if you do not attack, your hunger bar
will start depleting, which can be filled back up by hitting other mobs with
melee or your special skills", and the game's own front page leads with "attack
enemies to refill your hunger bar". *How much* a hit puts back is stated
nowhere, so `vitals::FOOD_PER_HIT` is `[APPROXIMATED]` at one food point --
half a shank, exactly what one drain tick takes off, so a hit buys back an
interval. It is the only number in the mechanic that is ours.

`Armor::apply` and `DamageKind::is_reduced_by_armor` in
`events/smash/src/module/damage.rs`; the drain, the starve tick and the refill
in `events/smash/src/module/vitals.rs`.

### Regeneration is health per second, and the wiki is not decisive about it

`[SHEET]` + `[WIKI]` Every kit carries a regeneration rate and they are all
different: 0.40 for Creeper down to 0.15 for Blaze. The wiki lists them as bare
"0.35 Regen Per Second" with no unit.

`[INFERRED]` The unit implemented is health points -- half-hearts -- per second,
which is what the kit table below means by "Regen (HP/s)" and the unit the rest
of the crate counts health in. The wiki's Slime page glosses Slime's 0.35 as
"regenerating 1 heart in just four seconds", and that fits neither reading:
half-hearts per second gives 5.7 s to the heart, hearts per second gives 2.9 s.
One loose sentence is not enough to move the unit, and the disagreement is
recorded rather than resolved.

Nothing in any source gates regeneration on being out of combat, and the design
argues against it: health *is* the knockback percentage here, so a regeneration
that stopped during a fight would be a percentage that only ever went up.
`vitals::VitalsModule` therefore heals continuously, in combat and out, and
stops for exactly one condition -- zero health, because the kill plane reads
`Health::is_dead` in the same tick and a heal off zero would cancel deaths.

---

## Four lives, and the void takes the credit

`[SOURCE]` `SuperSmash.java:110`: `private static final int MAX_LIVES = 4;`.
The frequent claim of three comes from Mineplex's own mode description saying
"each player has 3 respawns", which is the same number counted differently. Both
the wiki and the current [player guide][guide] say four.

`[SOURCE]` Death flow: lose a life, spend `DeathSpectateSecs = 4` seconds as a
spectator, respawn with `RESPAWN_INVUL = 1500` ms of immunity that is cancelled
early the instant you use an item. Lose your last and you are `PlayerState.OUT`,
a permanent spectator, with your placement recorded in reverse elimination
order.

`[SOURCE]` Life colours on the scoreboard: four or more green, three yellow, two
gold, one red, zero grey. Above fourteen players the per-player list collapses
to "Players Alive" and "Players Dead".

`[SOURCE]` Kill credit for a void death is the subtle part. The void injects
5000 damage attributed to *the game*, not to a player, so the credit comes from
the combat log instead: whoever hit you last, if they did it recently enough.
Mineplex kept a full combat log with assists.

`[INFERRED]` This implementation keeps only the last hit, as a
`(LastHitBy, attacker)` relationship with a `LastHitAt` timestamp and a
ten-second window. Assists are not implemented. The window length is mine —
Mineplex's `CombatLog` expiry was not something I could read off.

`events/smash/src/module/lives.rs`, `events/smash/src/module/scoreboard.rs`.

---

## The map is a set of platforms and a kill plane

`[SOURCE]` There is no fixed void Y. `GameFlagManager.java:1028` kills anyone
below `WorldData.MinY`, which is read from each map's own configured minimum
corner. Water is equally lethal: the game sets `WorldWaterDamage = 1000`, so an
ocean under the platforms kills like a void does. Players are in adventure mode,
explosions regenerate terrain after 30 seconds.

`[WIKI]` Maps: Adrift, Alpine Ruins, Amplified, Ancient Islands, Apache, Ardan
Forest, Astron, Avialae, Caste, Desert, Extinction, Garden, Glacier, Hyrule
Modified, Mining Camp, Mushroom Islands, Oriental Gardens, Remote Islands,
Skylands, Swamp, Tribal Haven, Wasted Lands, Amazon.

`[INFERRED]` The `Arena` singleton here carries a name, a `kill_y` and a list of
spawn points, with a small hardcoded default. Real map loading is on the far
side of the hyperion seam.

---

## The Smash Crystal is a heal as much as an ultimate

`[SOURCE]` One spawns every 3–8 minutes (`now + 3 min + random × 5 min`), the
timer resetting when the previous one is *collected*. A beacon and a quartz ring
appear at a map-authored point, the crystal descends 120 blocks at 8 blocks per
second — about fifteen seconds — and whoever walks within 2 blocks of it gets a
nether star. Right-clicking the star fires the kit's ultimate.

`[SOURCE]` The non-obvious part: `SmashUltimate.activate()` calls
`player.setHealth(getMaxHealth())`. **The crystal fully heals you**, which given
the knockback model is a knockback reset — arguably worth more than the
ultimate.

`[changelog]` Durations are per kit and were cut in 2026: Iron Golem, Spider,
Slime, Squid, Witch, Wither Skeleton, Skeletal Horse, Pig, Blaze and Villager to
15 s; Enderman, Wolf, Zombie and Cow to 20 s.

**Not implemented.** Crystal spawning, the beacon and the descent are not built.
Ultimates are defined on every kit (`KitBuilder::ultimate`) and are granted
through the same `(Grants, ability)` relationship as anything else, so wiring
the crystal is adding one module that grants and later revokes that pair. See
"What is missing".

---

## Double jump is creative flight, abused

`[SOURCE]` `PerkDoubleJump` sets `setAllowFlight(true)` for anyone standing on a
block, cancels the resulting `PlayerToggleFlightEvent`, and applies a velocity
by hand. **Touching the ground is what re-arms it.** Two modes: uncontrolled
(mostly vertical, the default) and controlled (goes exactly where you look —
Wolf and Spider). Power and height limit are per-kit sheet values.

`[guide]` The known "triple jump" — vanilla jump, double jump before landing,
get another — is an artefact of the loose ground check, and the relaunch's
changelog lists fixes in exactly that area.

Built, and built the same way. `events/smash/src/module/jump.rs` keeps
`Flight::Armed` on any playing player who is off the ground with a jump left,
pushed across the seam only when the answer changes; `src/mirror.rs` copies the
host's flying bit onto `Flying`; and seeing it true is what spends a
`JumpsLeft`, clears the flight and adds the velocity. No client mod, because
the double tap a vanilla client already sends is the input.

Two departures from the citation, both deliberate. `UtilAction.velocity`'s
`yMax` of 1.0 is not implemented: every uncontrolled kit declares a
`jump_power` of at least 0.9, so a ceiling there would clamp all twelve of them
to the same jump and the per-kit number would stop meaning anything. And the
"triple jump" is refused rather than reproduced — a press that arrives while
the player is standing on something spends nothing.

`jump_count` is on `KitStats` beside `jump_power`, because the Chicken's
`[VERIFIED]` eight flaps are a per-kit number and not a constant.

---

## The kits

Four are implemented, chosen to be structurally different rather than to be the
four most popular: a melee kit, a ranged kit, a movement kit and a kit built
around a resource bar. All numbers `[SHEET]` unless marked.

| | Iron Golem | Skeleton | Enderman | Slime |
|---|---|---|---|---|
| Role | melee | ranged | movement | resource |
| Melee damage | 7 | 5 | 7 | 6 |
| Armour (points / reduction) | 16 / 64% | 12 / 48% | 12 / 48% | 8 / 32% |
| Knockback taken | 100% | 125% | 130% | 175% |
| Regen (HP/s) | 0.20 | 0.20 | 0.25 | 0.35 |
| Hunger interval | 7.75 s | 7 s | 7.75 s | 7 s |
| Jump power | 0.9 | 0.9 | 0.9 | 1.2 |
| Energy bar | — | — | — | yes |

**Iron Golem** — Fissure (iron axe, 8 s, grounded, damage `4 + column` over
fourteen blocks), Iron Hook (iron pickaxe, 8 s, `|velocity| × 4`, pulls the
victim in at `velocity(2, yBase 0.8, yMax 1.5)`), Seismic Slam (iron shovel,
7 s, `10 × falloff + 0.5` over radius 8 at a 2.4 multiplier). Ultimate:
Earthquake, 16 s. `[SOURCE]` It also carries permanent Slowness I as the price
of the armour.

**Skeleton** — Barrage (bow, hold and release, up to five arrows, 6 damage each
at a 1.5 multiplier, 1000 ms to the first arrow and 300 ms per arrow after),
Bone Explosion (iron axe, 10 s, 6 damage over radius 7 at a **2.5** multiplier —
the highest of any starting ability in the game), Roped Arrow (5 s, fires and
drags you after it). Ultimate: Arrow Storm, 8 s.

**Enderman** — Block Toss (iron sword, 1.2 s charge, 2 s cooldown, `min(9,
|velocity| × 8)` at a 2.5 multiplier), Blink (iron axe, 7 s, 16 blocks
instantly), Teleport (crouch-charged, 5 s, up to 100 blocks, cancelled by being
hit). Ultimate: Dragon Rider, 30 s. Two of its three abilities do no damage,
which is the point.

**Slime** — Slime Rocket (iron sword, up to 3 s of charge, 6 s cooldown, size
`max(1, floor(chargeSeconds))` and damage `3 + 3 × size`), Slime Slam (iron axe,
6 s, 7 damage at a 2.0 multiplier, with a quarter of both recoiling onto you).
Ultimate: Giga Slime, 19 s. Energy regenerates at `0.004` per 49 ms tick, which
is the `0.0816`/s in the code.

### Numbers I could not source

- **Magma Cube's Meteor Shower duration.** Absent from the sheet entirely, and
  the Java default is `0`, which makes the ability fire exactly one meteor.
  Either the dump or the production data is broken. Not implemented.
- **Per-kit hitboxes.** There are none: the collision box stays the vanilla
  0.6 × 1.8 player box for every kit and the mob shape is a client-side
  disguise. The wiki's hitbox figures for four kits are the *disguise* sizes.
- **Exact minimum and maximum lobby size.** Server-group configuration, not in
  the tree. The wiki and a staff forum reply both say four to start; the
  scoreboard's collapse path proves lobbies well above fourteen existed.
  `LobbyConfig` defaults to a minimum of 4 and a full lobby of 8, and the 8 is
  `[INFERRED]`.
- **Kill credit window.** `[INFERRED]` at ten seconds.

### Where the sources contradict each other

Recorded rather than silently resolved. In each case the sheet or the Java is
what got implemented.

| | Source | Wiki |
|---|---|---|
| Skeleton Barrage max arrows | 5 | 6 |
| Creeper Explode damage | 20 / 30 in smash | 18 |
| Wolf Frenzy duration | 30 s | 20 s |
| Snowman Snow Turret count | 1 | 3 |
| Guardian Thorns | 66% | 15% / 34% |

`[SOURCE]` Two of Mineplex's own lore strings contradict their code: Magma
Cube's "Fuel the Fire" says "−15% knockback per kill" while the code applies a
`stacks × 0.15` *multiplier* (an 85% reduction at one stack), and Snowman's
Arctic Aura says "60% knockback" while applying a `0.4` multiplier, which is the
same thing said backwards.

---

## Architecture: the game is an import list

The requirement was aggressive modularity, using flecs `Module` as the unit of
composition. `SmashModule::module` is the whole composition root:

```rust
world.import::<PlayerModule>();
world.import::<KnockbackModule>();
world.import::<DamageModule>();
world.import::<AbilityModule>();
world.import::<KitModule>();
world.import::<ArenaModule>();
world.import::<LivesModule>();
world.import::<ProjectileModule>();
world.import::<LobbyModule>();
world.import::<ScoreboardModule>();
world.import::<StockKits>();
```

Every one of those is a `#[derive(Component)] struct` with an `impl Module`, and
so is every kit. A deployment that wants no lobby imports everything but
`LobbyModule`.

### A kit is data, not code

The acceptance test set for "modular" was: **adding a kit is one new module file
plus one `world.import`, touching no existing match statement, no enum and no
dispatch table.** It is met, and `events/smash/tests/modularity.rs` is the
proof: it defines a complete kit — stats, three abilities with three different
activation shapes, an ultimate and a passive that hooks the damage pipeline —
from *outside the crate*, using only the public API, and then walks every `.rs`
file under `src/` asserting that none of them mentions it.

The mechanisms that make that true:

**Abilities are entities, not enum variants.** An ability entity carries its own
`Slot`, `Item`, `CooldownSpec`, `Cooldown` and an `OnActivate(fn(&Cast<'_>))`.
There is exactly one activation path in the game — `ability::activate` — and it
never names a kit, because behaviour is a component it reads rather than a
branch it takes.

**The hotbar layout belongs to the kit, not to each ability.** `AbilitySpec`
used to carry a `slot: u8` and each kit file filled it in by hand. Nothing can
check a slot number on its own: `slot: 1` is wrong only in relation to the rest
of its kit, and twelve of the fifteen kits numbered from 1, which left slot 0
empty on all of them. That is the key a vanilla client has selected when it
spawns, so twelve kits handed a player a bar whose first ability could not be
fired until they scrolled. Every other gate stayed green, because every ability
really was present and reachable.

`KitBuilder::ability` now hands out slots in declaration order from 0 and
`KitBuilder::ultimate` always takes `ULTIMATE_SLOT`. The order a kit file
declares its abilities in *is* the layout, so the empty first key is
unreachable rather than merely absent today. `events/smash/tests/hotbar.rs`
sweeps the registry for what is left, and `nix run .#smash-hotbar-e2e` reads
the inventory packets a real client is sent for each of the fifteen kits.

**Behaviour is a bare `fn` pointer, not a `Box<dyn Fn>`.** Activation is rare
enough that one indirect call is free, and a boxed closure would put an
allocation and a second pointer chase into a path a kit author will eventually
call from a per-tick system.

**Kits are prefabs.** `kit::define` builds a prefab with `KitStats` and a child
prefab per ability; `kit::apply` instantiates it onto a player, which copies the
stats and creates that player's *own* ability entities with their own cooldowns.
`tests/modularity.rs::cooldowns_are_per_player_not_per_kit` holds that line.

**Relationships instead of parallel maps.** `(Grants, ability)` on a player,
`(Playing, kit)` marked `flecs::Exclusive` so selecting a new kit removes the old
edge for free, `(LastHitBy, attacker)` also exclusive because "who gets the kill"
has one answer, `(FiredBy, shooter)` on projectiles. Using flecs relationships
rather than `Option<Entity>` fields means an entity being destroyed cleans up its
edges, so a disconnect mid-fight cannot leave a dangling attacker id.

**Component traits carry the invariants.** `Player` is declared
`(flecs::With, Position)`, `(flecs::With, Health)` and so on — and each *module*
adds the ones it owns, so `DamageModule` is what says every player has `Armor`
and `LivesModule` is what says every player has `Lives`. `KnockbackModel`,
`Arena`, `Lobby`, `LobbyConfig` and `MatchClock` are `flecs::Singleton`.

**Observers, not polling.** Damage, knockback, death and elimination are all
observers on payload events. Nothing polls for "did someone die".

**The registry is a query.** `kit::registry` is
`world.query::<()>().with(Kit).with(flecs::Prefab)`, and `kit::by_name` matches
on the `KitName` component rather than a path lookup — deliberately, because a
kit prefab created inside its module's scope has the path
`smash::kits::Skeleton::Skeleton`, and matching on the component means a kit's
name does not depend on where its module chose to live.

### Two events, so module order does not matter

`Damaged` lowers health. Only once that has landed is `Smashed` emitted, so the
knockback of a hit is computed against the health that hit left behind. If both
lived on one event the damage and knockback modules would have to agree on
observer registration order — exactly the implicit coupling that stops an import
list being reorderable.

### The one sharp edge

flecs matches an observer when the **emitted id** matches one of the observer's
terms, not when the observer's terms merely happen to be present on the entity.
So every game event is emitted tagged with `Player` (`player::notify`) and every
observer for one must name `Player` as a term. Leave it off and the observer
silently never fires.

This is documented on `player::notify` with an example, and the alternatives
were worse: tagging with the query's first component couples emitters to
observer shapes, and tagging with every component on the entity makes every
emit walk the archetype. It remains the one thing in this design that fails
quietly, and it is the first thing I would want a compile-time check for.

### What resisted being modular

Honest list.

- **`DamageKind` is an enum with a `match` in it.** Creeper's Lightning Shield
  arms only against non-melee and Guardian's Thorns reduces only projectiles, so
  kits must be able to ask where a hit came from. This is a closed taxonomy of
  the *engine*, not an open set of kits, and no kit extends it — but it is a
  match statement and I am not going to pretend otherwise.
- **The seam is an enum-shaped closed set too.** `Server` has nine methods.
  Adding a tenth means editing every implementation. That is the correct
  trade for a boundary, but it is not extensible and should not be.
- **`Cue` is a fixed list of six.** A kit wanting a genuinely new particle
  effect has to add a variant. Making it an open `&'static str` would have
  pushed the closed set onto the adapter instead of removing it.
- **Ability activation shapes are a fixed three**: tap, hold-and-release, and
  passive-via-observer. A kit needing a fourth — Mineplex's crouch-charged
  Teleport is arguably one — expresses it as a hold-and-release, which is not
  quite honest. Enderman's Teleport is modelled that way and marked as such.

---

## Performance notes

Judged against readability at each step, and where they conflicted the choice is
recorded.

**Reads do not cross the `Server` seam.** Position, rotation and ground state
are mirror components written by the adapter once per tick, so the per-tick hot
paths — the cooldown tick, the arena bounds check, projectile integration — are
plain component iteration with no virtual calls. Only writes go through the
`Server` trait, and writes happen on hit, on death and on kit change, never per
entity per tick.

**The one read that does cross a seam is terrain**, and it has its own:
`BlockWorld` in `src/module/blocks.rs`, one method, defaulting to `OpenAir`.
Terrain is the case the mirror cannot serve — millions of blocks, a handful
looked at per tick, and an authoritative copy that already exists on the host,
so copying it would be maintaining a second one that drifts the moment anybody
places a block. It is a separate trait rather than a tenth `Server` method
because `Server` is a list of things the game asks the host to *do*, and because
a default of "nothing is solid" means every test that is not about terrain, and
the whole of the mock, needs no implementation at all. The cost is one virtual
call per projectile per tick, and the call answers the whole segment rather than
one block, so the traversal stays in `geometry::sweep` where the host's block
store and the tests' `Cubes` share it.

**Ability behaviour is a function pointer.** Zero allocation, one indirect call
per activation.

**Singletons are read once per call site, not once per entity.** Where two are
needed together they are fetched in one `world.get::<(&A, &B)>`.

**`splash_at` collects victims before hurting any of them.** Not a
micro-optimisation: the damage observers write components the query is reading,
and flecs catches that at runtime. The allocation is a `Vec` per ability
activation, which is the readable choice; a `SmallVec` would remove it, and if a
profile ever says it matters that is the change.

**The one place readability lost.** `resolve` keeps Mineplex's redundant
normalise-scale-measure round trip rather than the algebraically equal
`0.2 + 0.48 · K`, because being able to diff the constants against the original
Java is worth more than three floating-point operations on a path that runs once
per hit.

---

## What is missing

Stated plainly, because these are the parts nobody can see from the code.

1. **Only projectiles read the block world.** Projectiles now sweep their
   tick's travel against terrain and stop at the first surface, through the
   read seam in `src/module/blocks.rs`; the rest of the list here still does
   not. Fissure resolves its fourteen columns immediately instead of walking a
   block wall, and Enderman's Block Toss does not check that there is air above
   the block it picks up. Both want the same seam, which now exists. What a
   projectile does *about* an impact is also still one thing for every kind --
   it sticks and expires -- so a Sulphur Bomb that meets a wall stops there
   rather than detonating (ENG-12055).
2. **No Smash Crystal spawning.** The ultimates exist and are granted through the
   ordinary relationship; the beacon, the descent and the pickup are not built.
3. **Assists.** Only the last hit is tracked.
4. **Seventeen of the twenty-one kits.**
5. **Teams and the Dominate variant.**

Three things left this list on the same night and the list is shorter than the
three, because two of them were one entry. Health regeneration and the hunger
drain are `events/smash/src/module/vitals.rs` (ENG-11450); the double jump is
`events/smash/src/module/jump.rs` (ENG-11440).

What is still wrong, and is now wrong *visibly* rather than merely declared:
the per-kit `hunger_interval` values. The changelog puts Zombie, Snowman and
Wither Skeleton at 7 s and Spider, Wolf and Chicken at 6.25 s, and all six are
still on the 7.75 s default. That was dead data disagreeing with a changelog
while nothing read the field; it is a balance defect now that something does.
ENG-11463, filed rather than folded in.

---

## Wiring to hyperion, file by file

**Built.** The plan below is what was written, and it went in as planned: four
new files, no change to any module under `src/module/`. What follows records
both the plan and where reality differed, because the differences are the
interesting part.

`SmashModule` still depends on nothing but `flecs_ecs 0.2.2` and `glam`;
`SmashHost` is the game plus hyperion. The flecs migration has landed, so the
repository's pinned `nightly-2025-05-05` builds the crate and the ordinary
`nix run .#test` and `nix run .#lint` cover it.

### New: `events/smash/src/adapter.rs`

The only file that imports both `hyperion` and `smash`. Implements `Server`:

| `Server` method | hyperion |
|---|---|
| `add_velocity` | `Velocity` component on the player entity |
| `teleport` | `Position` plus a `PlayerPositionLookS2c` |
| `set_health` | `simulation::metadata::living_entity::Health` |
| `set_hotbar` | `hyperion_inventory` — set slots 0..9 |
| `send_message` / `broadcast` | `net::Compose::unicast` / `broadcast` with `agnostic::chat`; `Channel::ActionBar` and `Title` map to `OverlayMessageS2c` and `TitleS2c` |
| `set_sidebar` | scoreboard objective packets |
| `set_spectating` | game-mode change plus `Vanish` |
| `cue` | `agnostic::sound` and particle packets |

### New: `events/smash/src/mirror.rs`

One system, before everything else in the tick, copying hyperion state onto the
smash components: `Position`, `Velocity`, `Facing` from look direction,
`OnGround`. This is the read half of the seam and the reason no read is a trait
call.

### New: `events/smash/src/input.rs`

Turns hyperion events into the crate's entry points:

- interact / right-click → `ability::use_slot(player, held_slot)`
- release of a held item → `ability::release_slot(player, held_slot)`
- attack → `damage::hurt` with the attacker's `KitStats::melee_damage` and
  `DamageKind::Melee`
- a `Play` state entity appearing → add `Player` and a `PlayerId`
- disconnect → destruct the entity, which cleans up its relationships

### New: `events/smash/src/main.rs`

Mirrors `events/bedwars/src/main.rs`: clap and `envy` argument parsing, tracing
setup, jemalloc, `Crypto::new`, then build the world, `world.set(ServerHandle)`,
`world.import::<SmashModule>()`, and run.

One addition over bedwars: `--embedded-proxy <ADDR>`. `HyperionProxyModule`
runs a proxy inside the game-server process, and bedwars imports it
unconditionally, so a game server started next to the standalone proxy has both
racing for port 25565. The loser panics in a background tokio task and the
process carries on, which is a hard failure to read. Making it a flag lets the
`nix run .#smash` stack run the game server and the standalone proxy with no
race, while passing `--embedded-proxy` still gives the single-process deployed
shape when a deployment wants it.

### New: `events/smash/src/module/selector.rs`

Mineplex's waiting lobby put one mob per kit on a pedestal of coloured wool and
you right-clicked the mob you wanted to be. This is that: a ring of fifteen
podiums in the middle of the hub, each a wool block with the kit's own mob
standing on it, generated from the roster at boot so adding a kit adds a podium.

Everything about it is a relation. A podium *is* its `(Offers, kit)` edge and a
mob *is* its `(StandsOn, podium)` edge, so a click that arrives naming an entity
becomes a kit in two hops with no table keyed on entity id or block position.
Whether a mob is taken is a query for any player with `(Playing, thatKit)`,
derived on every call rather than stored, which is what makes a disconnect free
a mob with no cleanup code anywhere: the claim was the player's edge and the
player is gone.

**One player per mob.** Nothing found says whether Mineplex reserved a kit, and
since kits were bought per account with gems it would be a strange rule to have
shipped, so treat this as ours. The claim lasts the rest of the match without a
second rule saying so, because `lobby::choose` already refuses any kit change
once the match commits.

**How a player is told.** The wool goes green to red the moment somebody takes
the mob, which is the only channel that works in the last seconds of a countdown
when nobody is reading chat, and it says *which* mob is gone. The action bar
line explains a refusal that has already happened rather than delivering the
news. There is no sound yet; that belongs with the audio work.

Making the mob clickable took a change to the engine. hyperion routed no
entity-interaction packet, so `minecraft:interact` reached the dispatch table
and fell through it. It is routed now as `event::EntityInteract`, and since 26.2
split attacking out into `minecraft:attack` that packet means a right-click and
nothing else. Its body is hand-written in `packets/play/entity.rs`: the
extractor models the field list but marks the packet `complete: false` over one
statement inside the `Vec3` LP codec, which this crate already has.

### New: `events/smash/src/command.rs`

`/kit <name>` and `/kits`, through `hyperion-clap`. No longer the only way to
pick a kit, but still the one a screen reader can use and the one every gate
that is not testing the selector reaches for. A Minecraft command argument is
one whitespace-delimited token and half the kit names have a space in them, so
`/kit` matches on the name with case and punctuation discarded: `/kit irongolem`
finds `Iron Golem`.

`/podiums` answers with one JSON object per podium: which kit, where its mob
stands, what colour its wool is and who holds it. It exists so that
`tools/smash-selector.py` can right-click a podium without recomputing the ring
in Python, which would be a second copy of the geometry and would keep passing
after the real ring moved.

### Changed: `events/smash/Cargo.toml`

Add `hyperion`, `hyperion-inventory`, `hyperion-utils`, `valence_protocol`,
`glam` from the workspace, `clap`, `envy`, `tracing`, `tracing-subscriber`,
`tikv-jemallocator`. `flecs_ecs` moves to `{ workspace = true }` once the
workspace declares it.

### Changed: root `Cargo.toml`

Nothing further. `events/smash` is already in `members`.

### Not changed

Every module under `events/smash/src/module/` and every test. That is the point
of the seam: if wiring requires editing a game module, the seam was in the wrong
place. It held.

---

## What the wiring found

Four things the plan did not anticipate. Recorded because each one is a
property of the boundary rather than of the code either side of it.

**Writes cannot be applied where they are made.** `Server` is called from inside
flecs observers — from ability activation, from the damage pipeline, from the
lobby's phase transitions — and at those points the world is mid-iteration.
Taking a second mutable borrow of a component a running query is reading is a
runtime abort in flecs, not a compile error, so applying `add_velocity`
immediately would have been a crash waiting on a coincidence. The adapter
queues instead and drains once per tick in `PostUpdate`. The cost is one tick of
latency on knockback; the benefit is that the whole class of bug is unreachable.

**A read-back inside a command sees nothing.** `lobby::select_kit` calls
`kit::apply` and then `kit::hotbar` on the same player. In a bare world that
works, and the tests prove it does. Inside a hyperion command it does not: the
command runs inside a system, so every `add` `kit::apply` makes is deferred, and
`kit::hotbar` reads back a player who has not been granted anything yet. Every
kit selection produced an empty hotbar and therefore no abilities at all.

The adapter marks the player and rebuilds the hotbar from a system on a later
tick, when the deferred operations have committed. This is not a flaw in
`select_kit` so much as an unstated precondition — *it must not run inside a
deferred context* — and the honest fix would be to say so in its signature.

**Cooldowns are the only observable proof an input landed.** A right-click that
finds no ability in the slot returns `Ok(())` and says nothing, which is correct
but means a mis-wired input layer is indistinguishable from an empty hotbar.
Firing the same ability twice and watching for `That ability is recharging.` on
the action bar is what finally distinguished them.

**The host was not joinable at all.** Independent of this crate:
`egress::player_join`'s `player_joins` system, which is what sends the Login
packet, reads its work list off `Comms::skins_rx`, and nothing in the tree ever
sent on `skins_tx`. Every client authenticated and then waited on *Joining
world…* forever while the proxy counted it as connected. Fixed in
`crates/hyperion/src/ingress/mod.rs` by routing both the offline and the
Mojang-fetch skin paths through that channel.

### Build note

`flecs_ecs 0.2.2` declares an MSRV of 1.88, which the repository's pinned
`nightly-2025-05-05` satisfies, so `nix run .#test` and `nix run .#lint` cover
this crate with no special handling.

There is a live hazard with concurrent builds: `flecs_ecs_sys 0.2.1` writes
its generated bindings into the **shared crates.io registry checkout** rather
than `OUT_DIR`, so two builds with different feature sets corrupt each other's
`bindings.rs`. Details and the upstream report are in
[`flecs-rust-api-notes.md`](./flecs-rust-api-notes.md).

# CLAUDE.md

Hyperion is a flecs-ECS Minecraft game engine (Rust, edition 2024). This file
records the repo-wide conventions an agent or contributor must follow. Skills
under the harness cover general house style; the rules here are hyperion's own.

## flecs modules: separate component registration from behavior

**The rule.** A flecs module is one of two kinds, never both:

- A **registration module** ONLY registers components. It calls
  `world.component::<T>()` (with traits like `add_trait::<flecs::Singleton>()`,
  `.meta()`, `.member(...)`), declares relationships, and builds prefabs. It
  installs no systems, no observers, and no behavior. Installing a singleton's
  default value with `world.set::<T>(..)` belongs here too, right after `T` is
  registered, because a `set` of an unregistered component is the same class of
  bug as a `get`.
- A **behavior module** holds the systems and observers. It runs no
  `world.component::<T>()` of its own. Instead, for every component `T` it
  touches, it `world.import::<TRegistrationModule>()` the registration module
  that owns `T`. flecs runs an imported module exactly once and before the
  importer's body, so every component a system reads is registered before the
  system is declared.

**Why this exists (ENG-11000).** The workspace builds flecs with
`flecs_manual_registration` (`Cargo.toml`, `[workspace.dependencies.flecs_ecs]`),
so a component must be registered before first use. The guard that enforces
this is a flecs `ecs_assert`, and flecs asserts are compiled out of release
builds. So the two profiles disagree:

- **Debug / dev profile** (`nix run .#smash`, `cargo test`): first use of an
  unregistered component aborts with
  `ECS_INVALID_OPERATION: Component <T> is not registered with the world before usage`.
- **Release profile** (every e2e gate binary): the assert is gone, the C side
  registers the component lazily on first use, and the code reads fine.

ENG-11000 was exactly this gap. `WorldTimeModule` did
`world.set(WorldTime::default())` without first registering `WorldTime` as a
`Singleton`. Every release gate stayed green (including a new `world-time-e2e`),
while `nix run .#smash` crash-looped on boot in the dev profile. A
release-binary gate structurally cannot catch a dev-only debug assert, so the
structure of the code, not the test suite, is what has to make this impossible.

**How the convention makes the class impossible.** If a system uses `T`, its
behavior module imports `T`'s registration module, and flecs's module-import
DAG runs that registration before the system is declared. There is no ordering
a contributor can pick that registers `T` after a use of `T`, because "uses `T`"
and "imports the module that registers `T`" are the same edit. Use-before-
register stops being a thing you can forget and becomes a thing the import graph
forbids.

**Second benefit: composability.** Because registration carries no behavior, a
consumer can import the *components* (the types) without importing the
*behavior* (the systems). The smash mock, for example, can import a
component-registration module plus one chosen physics behavior module without
dragging in the whole host or egress layer. Splitting registration out is what
makes "give me the types but not the systems" expressible.

### Skeleton

```rust
use flecs_ecs::prelude::*;

// ---- the components this domain owns ----
#[derive(Component, Debug, Clone, Copy)]
pub struct Health(pub f32);

#[derive(Component, Debug, Default)]
pub struct DamageTuning { pub multiplier: f32 }

/// Registration module: types and traits only, no systems or observers.
/// Behavior modules import this; it imports nothing behavioral.
#[derive(Component)]
pub struct CombatComponentsModule;

impl Module for CombatComponentsModule {
    fn module(world: &World) {
        world.component::<Health>();
        // A singleton: register with the trait, then install its default in
        // the same place. `add_trait::<flecs::Singleton>()` is load-bearing --
        // a bare `world.set` stores the value but never registers the type, and
        // that is the ENG-11000 abort in a dev build.
        world
            .component::<DamageTuning>()
            .add_trait::<flecs::Singleton>();
        world.set(DamageTuning::default());
    }
}

/// Behavior module: systems and observers. It registers no components; it
/// imports the registration module for every component it reads or writes.
#[derive(Component)]
pub struct CombatModule;

impl Module for CombatModule {
    fn module(world: &World) {
        world.import::<CombatComponentsModule>();

        system!("apply_damage", world, &mut Health, &DamageTuning($))
            .kind(id::<flecs::pipeline::OnUpdate>())
            .each(|(health, tuning)| {
                health.0 -= tuning.multiplier;
            });
    }
}
```

### The import-DAG ordering rule

- **Behavior imports registration.** Every behavior module begins by importing
  the registration module(s) for the components its systems and observers
  touch. Import the ones you use even if you believe a parent already imported
  them; flecs dedupes imports, so an extra edge is free and a missing one is a
  dev-profile crash.
- **Registration imports nothing behavioral.** A registration module may import
  another registration module (to pull in a component it builds on), but it
  never imports a behavior module. Keep the registration layer a pure DAG of
  types so importing "the components" can never drag in a system.
- **Register a relation before anything points at it.** A relationship or tag
  used in a `With`/`Suggests`-style trait must be a registered entity first, so
  its `world.component::<Rel>()` goes in the registration module ahead of any
  trait that references it.
- **Core singletons live in `HyperionCore`.** Cross-cutting singletons every
  event needs (`SpatialIndex`, `WorldTime`) are imported directly from
  `HyperionCore` in `crates/hyperion/src/lib.rs`, not nested inside one event's
  module chain, so they exist for every event before anyone joins.

### Adding a new component or system

- **New component `T` in an existing domain:** add `world.component::<T>()`
  (plus any `.meta()`/`Singleton`/relationship traits) to that domain's
  registration module. If `T` is a singleton with a default, `world.set(T::..)`
  right after, in the same module. Do not register it from a system's module.
- **New system that touches `T`:** put it in the domain's behavior module and
  make sure that module imports the registration module owning `T`. If `T`
  lives in another domain, add `world.import::<ThatDomainComponentsModule>()`
  to the top of your behavior module.
- **New domain:** create both modules, `FooComponentsModule` and `FooModule`,
  with `FooModule` importing `FooComponentsModule`. Wire `FooModule` into its
  parent's import chain (an event's top module, or `HyperionCore` for core
  simulation).
- **Verify in the dev profile, not just a gate.** A release e2e gate cannot see
  a use-before-register. Boot `nix run .#smash` (dev profile) or run a
  `cargo test` that imports your module before trusting it. Reverting a
  registration should reproduce the `ECS_INVALID_OPERATION` abort; that is the
  guard proving it works.

## Mirroring host state: mirror the level, never compute an edge

**The rule.** A system that copies host state onto a game component copies the
*value*. It does not compute "this just became true". If a consumer needs an
edge, either keep the memory somewhere that is written after the host state is,
or mirror the level and make the consumer idempotent.

**Why this exists (ENG-11440).** `events/smash/src/mirror.rs` runs in `OnLoad`.
hyperion decodes the tick's packets in `OnUpdate` (`ingress/decode.rs`) and
copies `is_flying` into `MovementTracking::last_tick_flying` in `PreStore`
(`egress/sync_entity_state.rs`) -- both strictly after. So
`bit && !hyperions_previous_bit`, evaluated in the mirror, is **always false**:
the packet lands after the mirror has run, and by its next run hyperion's own
"previous" has caught up. There is no point in the tick where the mirror can
see the two disagree.

**Why the test suite cannot catch it.** Every test under `events/smash/tests/`
drives the mirrored component directly, because the mirror is host-side and a
mock world has no host. A mirror that can only ever write `false` is therefore
invisible to all of them: the double jump shipped eleven passing unit tests and
a green contract while doing nothing at all on a real client. Only
`Match.prove_double_jump` in `tools/smash-match.py` -- a real client
double-tapping jump and getting no impulse -- showed it.

**What to do.** Mirror the level, and put the idempotence in the consumer.
`Flying` is the host's flying bit copied verbatim; `smash::Jump` answers a press
by clearing that bit through the seam in the same tick, and refuses a press from
a player standing on the ground. One double tap is one jump because of those two
properties, not because the mirror was clever.

**When you add a mirror of a host bit, gate it with a real client.** A Rust test
that sets the component proves the consumer, not the mirror. The two are
different code and only one of them has this hazard.

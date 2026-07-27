# Notes on the `flecs_ecs` Rust API

Written while building `events/smash/` against [`flecs_ecs`][repo] 0.2.2. Every
item here cost real time; each says what happened, what was changed locally, and
what the upstream fix would be.

The local changes all live in `events/smash/src/flecs_ext.rs` as extension
traits over upstream types — which is the shape an upstream patch would take as
an inherent method — and every one of them is zero-cost: no allocation, no
boxing, no extra indirection over the upstream call.

[repo]: https://github.com/Indra-db/Flecs-Rust

---

## Version choice: whatever the workspace pins

This crate takes `flecs_ecs = { workspace = true }` and nothing else. Two
`flecs_ecs` crates in one process would be two component registries and two
copies of flecs C, so the version is not smash's to choose.

The original note here argued for crates.io 0.2.2 on the grounds that the
in-flight `refactor/flecs` branch pinned it and building against anything else
would mean reconciling two versions later. That reasoning still holds; the
answer changed because the workspace moved. It now pins upstream `main`, for
the unreleased `!Send` work this note anticipated: `World` and `Query` are
`!Send` with a `QueryHandle` for the cross-thread case, which let hyperion
delete a hand-rolled `unsafe impl Send`. See the comment on
`[workspace.dependencies.flecs_ecs]` in the root `Cargo.toml` for the pin and
the upstream fix it carries.

---

## Upstream bug 1: `flecs_ecs_sys` writes generated bindings into the shared registry checkout

**Severity: this corrupts unrelated builds on the same machine.**

`flecs_ecs_sys-0.2.1/build.rs:220`:

```rust
let crate_root: PathBuf = env::var("CARGO_MANIFEST_DIR").unwrap().into();
bindings
    .write_to_file(crate_root.join("src/bindings.rs"))
    .unwrap();
```

`CARGO_MANIFEST_DIR` for a registry dependency is
`~/.cargo/registry/src/index.crates.io-*/flecs_ecs_sys-0.2.1`, which is **shared
by every build on the machine**. The generated content depends on the enabled
features, so two crates depending on `flecs_ecs` with different feature sets
overwrite each other's `bindings.rs`, and whichever build reads it second fails
with dozens of errors that point into the registry:

```
error[E0432]: unresolved import `crate::ecs_app_desc_t`
error[E0412]: cannot find type `EcsUnit` in this scope
  --> ~/.cargo/registry/src/index.crates.io-*/flecs_ecs_sys-0.2.1/src/mbindings.rs:337:22
```

This happened here for real: another agent was building the `refactor/flecs`
branch (which enables `flecs_manual_registration`) at the same time, and my
build failed twice with completely unrelated-looking errors before I found the
cause. It also means an interrupted build can leave a truncated `bindings.rs`
that poisons every later build until the crate is re-extracted.

**Fix.** Write to `OUT_DIR` and `include!` it, which is what every other
`-sys` crate does:

```diff
-    let crate_root: PathBuf = env::var("CARGO_MANIFEST_DIR").unwrap().into();
+    let out_dir: PathBuf = env::var("OUT_DIR").unwrap().into();
     bindings
-        .write_to_file(crate_root.join("src/bindings.rs"))
+        .write_to_file(out_dir.join("bindings.rs"))
         .unwrap();
```

```diff
 // src/lib.rs
-mod bindings;
+mod bindings {
+    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
+}
```

**Workaround carried here.** A private `CARGO_HOME` for this crate's builds. It
is not a fix and should be dropped the moment upstream lands one.

---

## Upstream bug 2: `EntityView::emit` silently does nothing for world observers

**Severity: silent no-op in the most natural spelling.**

This is the one that cost the most. `entity_view_const.rs:2347`:

```rust
pub fn emit<T: ComponentId>(self, event: &T) {
    self.world().event().entity(self).emit(event);
}
```

No id is ever set. flecs matches an observer when the emitted id matches one of
the observer's terms, so:

```rust
world.observer::<Damaged, (&mut Health, &Armor)>().each_iter(..);
victim.emit(&Damaged { .. });      // observer never fires. no error, no warning.
```

The observer simply does not run. Nothing reports anything. It only works if the
observer's query is `()` with an explicit `.with(id::<flecs::Any>())`, which is
what the one upstream example that uses `emit` happens to do — so the example
does not reveal the limitation.

Verified by probe:

```
-- plain entity.emit --                                   health = Health(20.0)   ← nothing happened
-- world.event().add(Health).entity().emit --   MULTI fired  health = Health(15.0)
-- world.event().add(Health).add(Armor).emit -- MULTI fired  health = Health(10.0)
```

A second probe established the matching rule precisely: **any** term counts, data
or filter, so `.with(Player::id())` on the observer plus `.add(Player::id())` on
the emit is sufficient.

**Local fix.** `EntityViewExt::emit_about::<Subject, E>`:

```rust
fn emit_about<Subject: ComponentId, E: ComponentId>(self, event: &E) {
    self.world().event().add(Subject::id()).entity(self).emit(event);
}
```

**Proposed upstream.** Either of:

1. Make `emit` default the id set to the entity's own type, so an observer on
   any component the entity has will match. Costs an archetype walk per emit,
   but emits are rare and a silent no-op is worse.
2. Keep the behaviour and make it loud: document it on `emit` and add a
   `debug_assert` when an event is emitted with an empty id set and there exists
   at least one observer for that event type with a non-empty query.

Option 2 is cheaper and is what I would send first.

---

## Upstream bug 3: `WorldRef`'s constructors return views borrowed from the borrow

`world/entity_view.rs:168`:

```rust
pub fn entity_from_id(&self, id: impl Into<Entity>) -> EntityView<'_> {
```

`WorldRef<'a>` is `Copy` and is itself just a borrow of the world, so tying the
result to `&self` is strictly more restrictive than it needs to be. The effect
is that this extremely ordinary line does not compile:

```rust
found.map(|id| player.world().entity_from_id(id))
//             ^^^^^^^^^^^^^^ returns a value referencing data owned by the
//                            current function
```

`WorldRef::entity()` has the same shape, so any helper that creates an entity
and returns it — every spawn function in a game — hits it too.

The lifetime-correct constructor already exists and is public:
`EntityView::new_from(world: impl WorldProvider<'a>, id) -> EntityView<'a>`. The
`&self` methods are just not written in terms of it.

**Local fix.** `WorldRefExt::entity_at` and `WorldRefExt::new_entity`.

**Proposed upstream.** Take `self` by value:

```diff
-pub fn entity_from_id(&self, id: impl Into<Entity>) -> EntityView<'_> {
+pub fn entity_from_id(self, id: impl Into<Entity>) -> EntityView<'a> {
```

`WorldRef` is `Copy`, so this is source-compatible at every call site.

---

## Upstream bug 4: `each_target` hands out a view that cannot escape the closure

```rust
player.each_target(Grants, |ability| {
    found = Some(ability);   // error: `ability` escapes the closure body here
});
```

The target ids are plain integers read out of the entity's type; nothing about
them is borrowed from the closure. But the `EntityView` handed in is tied to the
callback's frame, so every "find the target matching a predicate" turns into
collecting bare `Entity` ids and re-resolving them afterwards. Ability lookup by
hotbar slot runs on every right-click, so this is not a rare shape.

**Local fix.** `EntityViewExt::each_target_view` and `EntityViewExt::find_target`.
`find_target` is the one that gets used; `granted_in_slot` is now three lines.

**Proposed upstream.** Give the callback `EntityView<'a>` where `'a` is the
world's lifetime, exactly as `each_target_view` does. It is a signature change
on a callback parameter, so it is source-compatible for closures that ignore the
lifetime and strictly more permissive for those that do not.

---

## Papercut: a tag used as a data term fails a const assertion with no mention of the tag

```rust
world.observer::<UseSlot, &Player>()   // Player is a ZST tag
```

produces five screens of:

```
error[E0080]: evaluation of `<flecs_ecs::core::ObserverBuilder<'_, UseSlot, &Player>
  as SystemAPI<'_, UseSlot, &Player>>::each_iter_internal::<{closure@...}, true>::{constant#1}` failed
error[E0080]: evaluation of `flecs_ecs::core::table::field::flecs_field::<Player>::{constant#1}` failed
```

The diagnosis — *a zero-sized component has no data to fetch, so name it as a
filter term with `.with(T::id())` instead* — appears nowhere in the output.

**Proposed upstream.** The const assertions already exist; give them messages.
`const { assert!(size_of::<T>() != 0, "..."); }` renders its message in the
E0080. Something like: *"`{T}` is a zero-sized tag and has no data to fetch. Use
`.with({T}::id())` as a filter term instead of `&{T}` as a data term."*

That is a one-line change per assertion and it turns a five-minute confusion
into a five-second one.

---

## Documentation hazard: `flecs_ecs/tests/docs/**` is never compiled

`tests/docs/main.rs` declares only `pub mod common_test;`. Every other file in
that directory — `relationships.rs`, `prefabsmanual.rs`, `systems.rs`,
`queries.rs` — is dead. `tests/docs/relationships.rs:114` calls
`bob.target_for(...)`, which does not exist on either 0.2.2 or `main`, and
nothing notices.

Anyone treating that directory as worked examples will write code that does not
compile. `examples/flecs/**` *is* a real target with `test = true` and is the
directory to trust.

**Proposed upstream.** Either wire the modules into `main.rs` and fix the
fallout, or delete the directory. A stale example is worse than no example.

---

## Things that are right, and worth saying

**The runtime borrow tracker is excellent and caught two real aliasing bugs.**

```
Cannot increment read: write already set for component: #41 | Health
Cannot set write: reads already present or write already set for component: #41 | Health
```

Both were genuine: an observer holding `&mut Health` from its query while
emitting an event whose observer wanted `&Health`, and a system iterating
`&Health` while calling something that killed the player. Both are real
soundness problems in an ECS that a compile-time-only checker cannot see, and
both were reported at exactly the right moment.

The one improvement I would ask for: the message names the component but not
the *two* systems or observers involved. It already has the names —
`system_named` and `observer_named` exist precisely so entities have them — so
adding "held by `smash::apply_damage`, requested by `smash::apply_knockback`"
would take the diagnosis from a bisect to a read.

**Component traits are genuinely better than the alternative.** `(flecs::With, T)`
letting each *module* declare the components it needs on `Player` — `DamageModule`
adds `Armor`, `LivesModule` adds `Lives` — is something bevy has no equivalent
of, and it removed an entire class of "the query silently does not match" bug
without a central registration list.

**`flecs::Exclusive` on a relationship is exactly the right primitive** for
`(Playing, kit)` and `(LastHitBy, attacker)`. Selecting a new kit removes the
old edge atomically instead of needing a clear-then-set pair that can be
interrupted.

---

## Changes considered and rejected

**A typed DSL over the query builder.** Considered wrapping the builder chain in
something shorter. Rejected: it hides which flecs feature is being used, which
is the opposite of what this crate is for, and every wrapper is another thing to
keep in sync with upstream.

**`Box<dyn Fn>` for ability behaviour instead of `fn`.** Would let abilities
capture. Rejected on cost: an allocation and a second pointer chase, in a path a
kit author will eventually call per tick, to buy a capability no kit needed —
everything an ability wants is reachable through `Cast`.

**An extension trait for `Copy` singleton reads.** `world.get::<&T>(|t| *t)` is
noisy. Rejected because `world.cloned::<&T>()` already exists and does exactly
this; the friction was that I did not find it, not that it is missing.

**`SmallVec` in `splash_at`.** The collect-then-act pattern allocates a `Vec` per
ability activation. Rejected for now: activations are rare, and the readable
version should stay until a profile disagrees. Recorded here so the option is
findable.

**Patching `flecs_ecs` in-tree via `[patch.crates-io]`.** Would let the fixes
above be inherent methods rather than extension traits. Rejected: it means
editing the root `Cargo.toml` beyond adding a workspace member, it pins the
whole workspace to a fork, and hyperion's own flecs migration is landing on the
crates.io release. Extension traits give the same ergonomics with none of that,
and the diff to upstream stays legible.

---

## Summary of local additions

`events/smash/src/flecs_ext.rs`, ~110 lines, all `#[inline]`:

| Item | Fixes |
|---|---|
| `WorldRefExt::entity_at` | bug 3 — `entity_from_id` lifetime |
| `WorldRefExt::new_entity` | bug 3 — `entity` lifetime |
| `EntityViewExt::each_target_view` | bug 4 — non-escaping target views |
| `EntityViewExt::find_target` | bug 4 — the common case of it |
| `EntityViewExt::emit_about` | bug 2 — `emit` silently not matching |

Ready to send upstream, in priority order: the `OUT_DIR` fix (bug 1, correctness
and affects everyone), the `emit` assertion or default (bug 2, silent
misbehaviour), the `self`-by-value constructors (bug 3, mechanical and
source-compatible), the `each_target` lifetime (bug 4), the const-assert
messages (papercut), and the dead `tests/docs` directory.

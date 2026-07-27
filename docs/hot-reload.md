# Hot reloading flecs game modules

Edit a game module, rebuild it, and see the change in a running server without
restarting or dropping players. When a component's memory layout changes in a way that
would make the stored bytes mean something different, the reload is **refused** and the
running world is left exactly as it was, unless the developer supplied a migration.

Refusing is the point. The alternative is not a crash; it is a server that keeps running
while every `Health` in the world quietly becomes `2.9e-44`.

- Crate: `crates/hyperion-hot-reload`
- Demo: `nix run .#hot-reload-demo`, source in `crates/hyperion-hot-reload-demo{,-module}`

## Prior art: everyone else either forbids layout changes or throws the state away

Four projects cover the design space. None of them diff a schema and refuse.

**`libloading`** is the `dlopen` wrapper everything else is built on. It loads a dylib and
hands back typed symbols. It has no opinion about state, types or safety beyond "this
symbol has the type you claimed". Taken: the loading mechanism itself.

**`hot-lib-reloader`** watches a dylib and re-`dlopen`s it, generating a proxy module so
calls transparently hit the newest build. Its documented limits are exactly the problem
this crate attacks:

> Types of structs and enums that are used in both the executable and library cannot be
> freely changed. If the layout of types differs you run into undefined behavior which
> will likely result in a crash.

Its escape hatch is to route mutable state through `serde_json::Value` so the layout the
compiler sees never changes. That works, and it costs you the ability to hold a typed
struct at all. Its README also names the ECS problem directly: crates relying on `TypeId`
"will expect the type/id mapping to be constant. After reloading, types will have
different ids". Taken: the warning, and the decision to make identity survive reloads by
keying on a stable string rather than on any id.

**`dexterous_developer`** is the closest prior art, and the only one that treats schema
evolution as a first-class feature. Its Bevy adapter serialises reloadable components and
resources with `rmp_serde` and deserialises them into the new build, "allowing you to
evolve their schemas so long as they are compatible with the de-serializer". It also lets
you mark entities to be destroyed on reload and reset resources to defaults.

That is genuinely schema evolution, but it is *implicit*: the policy is whatever
`rmp_serde` happens to accept. There is no diff, no statement of what changed, and no
point at which a developer is told "this change needs a migration". A field reorder that
serde tolerates passes silently; a change serde rejects surfaces as a decode error per
value, at apply time, after the old state is already gone. It also requires
`Serialize + Deserialize` on everything reloadable and pays a serialise/deserialise round
trip for all live state on every reload, including the reloads that changed nothing.
Taken: the conviction that schema evolution belongs in the reload path. Not taken:
implicit conversion in place of an explicit gate.

**Dioxus `subsecond`** is the most technically ambitious and the least applicable. Rather
than swapping a dylib it patches at function granularity through a jump table, so the
original executable is never modified and no memory is rewritten. It is also explicit that
structs are out of scope:

> Subsecond currently does not support hot-reloading of structs. This is because the
> generated code assumes a particular layout and alignment of the struct. If layout or
> alignment change and new functions are called referencing an old version of the struct,
> the program will crash.

Its stated mitigation is "re-instancing", which it delegates to the framework and defines
as throwing the old state out: "frameworks that implement subsecond patching properly will
throw out the old state". Dioxus does precisely that and rebuilds the UI from scratch. For
a UI that is free. For a Minecraft server it is the thing we are trying to avoid, since
the state *is* the game. Subsecond additionally only patches the tip crate, which rules it
out for game logic living in workspace crates. Taken: nothing structural; it is a good
argument that a jump table is the wrong tool when the state, not the code, is the asset.

**Where this design differs.** Every project above resolves a layout change by forbidding
it, converting it implicitly, or discarding state. This one makes the layout change the
unit of review: the reload either proves the bytes still mean the same thing, or applies a
migration the developer wrote for exactly the old layout the world actually holds, or
stops and says why.

## The manifest: a component tree read from a live world, not from source

Every build emits a module tree and a component tree, but not at build time. The manifest
is read out of a running flecs world, because that is the only place that knows what a
module *actually* registered, including anything it registered indirectly through a
dependency.

Reading it costs a throwaway world. On load, before the live world is touched:

1. `dlopen` the candidate and call its one exported entry point,
   `hyperion_hot_module`, which returns a `ModuleDescriptor`.
2. Check the `AbiToken` (below). Stop here if it fails.
3. Create a `World::new()`, sample it, run the module's registration into it, and diff the
   sample. What appeared is what this module registers.
4. Read a `ComponentSchema` for each new component entity.
5. Diff against the manifest of the build currently loaded, and decide.

The throwaway world is safe because `flecs_ecs` is built with the
`flecs_manual_registration` feature, which keeps component ids per-world rather than in a
process-global cache. Verified directly: the same type registered into a probe world and
then into a live world whose id allocation had moved on got id 33 and id 34 respectively,
and stores through the live world read back correctly. Without that feature the probe
would poison the live world's id mapping.

### Identity is the flecs symbol, never the path

`world.import::<ArenaModule>()` scopes a module's components underneath the module entity,
so `Health` is registered at the path `::demo_module::ArenaModule::Health` while its
*symbol* stays `demo_module::Health`. The symbol is `core::any::type_name`, it does not
move when a module is renamed or re-parented, and `ecs_lookup_symbol` resolves it.

The earlier attempt at this keyed on `type_name` but looked components up by path. The two
never matched, so it found nothing and reported an empty component tree on every load.

### Nested types are flattened, because a name comparison is not a layout comparison

A `ComponentSchema` records size, alignment, and a `Layout`:

| `Layout` | When | Compared by |
| --- | --- | --- |
| `Reflected(Vec<FieldSchema>)` | every leaf reduced to a flecs primitive | exact leaf list |
| `Enum(Vec<String>)` | fieldless enum | constants, in declaration order |
| `Opaque { declared_version }` | no reflection, developer vouched | the version integer |
| `Unknown` | no reflection, nobody vouched | never equal, not even to itself |

`FieldSchema` leaves carry a *dotted path and an absolute offset*: `inner.a: u16 @0`,
`inner.b: u16 @2`, `tail: u32 @4`. They are not the members flecs reports, which are one
level deep.

That flattening is the difference between a detector and a hazard. flecs reports a nested
member as a name and an offset — `inner: demo::Inner @0` — and says nothing about that
type's interior. Change `Inner` from `{a: u16, b: u16}` to `{a: u8, b: u8, c: u16}` and its
size, alignment and the enclosing member list are all unchanged, so a reader that records
the name compares the two builds as equal and reinterprets every byte past the first
field. `tests/reflection.rs` pins this with two structs that differ only inside a nested
type; with flattening disabled both reduce to the same `[(0, 4), (4, 4)]`.

Anything the walk cannot reduce to primitives makes the **whole component** `Unknown`
rather than partly checked. A partly checked component is the dangerous one: it compares
equal while the unchecked part changed underneath. Arrays, vectors, bitmasks, opaque
types, inline arrays with `count > 1`, and nesting past 16 levels all fail the walk.

## Migrations are ordinary Rust, checked against the world before a byte is read

`Health: u32 -> f32` in full:

```rust
hyperion_hot_reload::migration! {
    component: Health,
    from: { hp: u32 },
    with: |old| Health { hp: old.hp as f32 },
}
```

The `from:` block is not documentation. The macro turns it into a `ComponentSchema` built
from `size_of`, `align_of` and `offset_of!` on a struct declared with those exact field
types, and the gate compares that against what the running world reports **before** the
migration is allowed to read anything. A migration whose declared old layout does not match
the live one is refused as `MigrationDoesNotMatch`, because applying it would read the live
bytes at the wrong offsets — worse than having no migration at all.

Field types in a `from:` block must implement `ReflectedType`, which is implemented only
for flecs primitives. A non-primitive field is a compile error rather than a check the gate
cannot perform.

When a component has no reflection at all and cannot get any, the developer can vouch for
it by listing it in `opaque_versions` and bumping the integer whenever the interior
changes. This is the same bargain as NixOS's `system.stateVersion`: a hand-maintained
number standing in for a property the system cannot observe, load-bearing exactly to the
extent that a human keeps it honest.

### Applying a migration: harvest, tear down, re-register, write back

Changing a component's size changes its archetype layout, and flecs aborts the process with
`ECS_INVALID_COMPONENT_SIZE` if you re-register a component at a new size over the existing
entity. So the component entity is deleted and recreated, which means the old bytes have to
be copied out first. The order is fixed and each step exists to stop the next one observing
a half-updated world:

1. **Harvest.** For each planned migration, iterate the component and copy every instance's
   raw bytes into a `Vec<(Entity, Vec<u8>)>`.
2. **Tear down.** Delete the systems and observers the previous build registered; they are
   live function pointers into the dylib being replaced, and leaving them runs old and new
   code side by side. Remove the `flecs::Module` tag from the module entity — *remove*, not
   delete, because a module's components are its children and deleting it would take their
   data with it. Then delete the component entities being migrated.
3. **Re-register.** Run the new build's registration. `world.import` is idempotent and
   short-circuits on the module tag, which is why step 2 clears it; without that a reload
   registers nothing and reports success while the old code keeps running.
4. **Write back.** For each harvested row, run the developer's function into a fresh buffer
   and `ecs_ensure_id` it onto the entity.

**Cost.** Harvesting is one pass over the migrated component's archetypes and allocates
`instances × size` bytes; write-back is one `ecs_ensure_id` per entity, each of which is an
archetype move. For a world with thousands of entities carrying a changed component that is
thousands of archetype moves in one tick — a visible hitch, not a hang, and it happens only
for components that actually changed. Components that did not change are never touched and
never copied, which is the main advantage over serialising all live state on every reload.

**Failure modes.** The gate runs entirely before step 1, so a refusal cannot leave a
partial state. Past step 2 there is no rollback: if the new build's registration panics
midway, the module's systems are already gone and its components are half re-registered.
That window is real and is not currently guarded — see "Not done" below.

## Demonstration: three cases, one running world

`nix run .#hot-reload-demo` builds four versions of the demo module and drives a single
world through all of them. Cargo features stand in for editing the source. The world, its
three entities and their data are created once, before the first reload, and never rebuilt.

**Code-only change accepted, state intact.** `--features tuned` changes one constant in a
system body: regeneration goes from 1 to 5 per tick.

```
=== reload: .../v2-code-only.dylib ===
  ACCEPTED
    kept: ["hyperion_hot_reload_demo_module::Health", "hyperion_hot_reload_demo_module::Score"]
    added: []
    dropped: []
    instances rewritten: 0
  after one tick:
  entity 0: hp: u32 = 16 | points: i32 = 7
  entity 1: hp: u32 = 26 | points: i32 = 14
  entity 2: hp: u32 = 36 | points: i32 = 21
```

Before this reload the values were 11 / 21 / 31, so `+5` proves the new code is the code
running, and `points` unchanged at 7 / 14 / 21 proves the state was not rebuilt.

**Layout change refused.** `--features health-f32` changes `Health.hp` from `u32` to `f32`.
Same size, same alignment; only the leaf type differs.

```
=== reload: .../v3-layout-change.dylib ===
  REFUSED
refusing to reload module `arena`: 1 unmigrated schema change(s). The running world was not modified.

[1] component `hyperion_hot_reload_demo_module::Health` changed layout and no migration was supplied.
  was: hyperion_hot_reload_demo_module::Health (size 4, align 4, hash 94e44c51420a29bd) { hp: u32 @0 }
  now: hyperion_hot_reload_demo_module::Health (size 4, align 4, hash e65b7156d7651a92) { hp: f32 @0 }
  Add to the module:
    hyperion_hot_reload::migration! {
        component: Health,
        from: {
            hp: u32,
        },
        with: |old| Health { /* map each field */ },
    }

  world still running on the previous build:
  entity 0: hp: u32 = 21 | points: i32 = 7
```

The world kept ticking on the previous build throughout, still at `+5` per tick.

**Same change accepted once the migration exists.** `--features health-f32,migration` adds
the four lines from the stub.

```
=== reload: .../v4-with-migration.dylib ===
  ACCEPTED
    kept: ["hyperion_hot_reload_demo_module::Score"]
    migrated `hyperion_hot_reload_demo_module::Health`: 94e44c51420a29bd -> e65b7156d7651a92
    instances rewritten: 3
  after one tick:
  entity 0: hp: f32 = 22.0 | points: i32 = 7
  entity 1: hp: f32 = 32.0 | points: i32 = 14
  entity 2: hp: f32 = 42.0 | points: i32 = 21
```

The values verify the migration converted rather than reinterpreted. Health was 21 / 31 / 41
as `u32`; v4 regenerates 1.0 per tick, so 21 + 1 = 22.0 is correct. Had the bytes been
reinterpreted, `21u32` read as `f32` would be `2.9e-44`.

### One thing this demo hid for several runs

Every case above printed correctly while the process was **segfaulting at exit**. It was
invisible because the run was piped to `tail`, and a pipeline reports the exit status of
its last stage. Redirecting to a file and checking `$?` showed `139`, and lldb showed
`EXC_BAD_ACCESS` jumping to an address whose memory could not even be read -- a call into
an unmapped image.

The cause was `libloading::Library::drop` calling `dlclose`. The code carried a comment
saying the libraries were "deliberately never closed" while doing exactly the opposite, and
because `HotReloader` is declared after `World` it dropped first, unmapping the images that
the world's teardown was about to call into. The handles are now `mem::forget`-ed and the
demo exits 0.

This is the failure mode the whole design is aimed at, arriving through the back door:
output that looks entirely correct, with the corruption one stack frame later.

## What is safe, what is refused, and what is not caught

**Detected and allowed, with data kept:**

- a change to any function body, including system and observer implementations
- adding or removing systems and observers
- adding a component (nothing stored under it yet)
- removing a component (its data is dropped with it, and this is reported)
- a tag growing into a real component: there were no bytes per entity to misinterpret

**Detected and refused unless a migration exists:**

- a primitive field changing type at the same size, `u32 -> f32` and `u32 -> i32`
- any change to size or alignment
- fields added, removed, renamed or reordered, including compiler-chosen reordering,
  because offsets are read from the real layout rather than from declaration order
- a change inside a nested struct, at any depth up to 16
- an enum variant added, removed, renamed or reordered
- a component that carries no reflection and holds live data, refused as
  `UnprovableLayout` even when its size and alignment are unchanged
- a migration whose declared old layout does not match the running world
- a module built by a different rustc, or one that linked its own copy of the runtime

**Not caught. Read this list before trusting the gate.**

- **Semantic changes at identical layout.** `Health(u32)` in half-hearts becoming
  `Health(u32)` in whole hearts is invisible. No layout detector can see this; it needs a
  version bump the developer chooses to make.
- **Data-carrying enums.** `#[flecs(meta)]` reflects a fieldless enum as its constants and
  a payload enum as nothing useful. A payload type changing at constant size and unchanged
  variant names would not be caught if such an enum ever reflected as `Enum`. Payload enums
  in practice land in `Unknown` and are refused, but this rests on flecs' behaviour rather
  than on a check of ours.
- **Explicit discriminant values.** `enum Mode { Idle, Busy }` to `{ Idle, Busy = 5 }` has
  the same constant names in the same order. flecs 0.2.1 does not tag those child entities
  with `EcsConstant` in a way this crate reads, so only names and order are compared.
- **Pointers and handles.** A field holding a pointer, a `Box`, or an index into a
  collection owned by the old dylib is bit-identical across the reload and dangling
  afterwards. These land in `Unknown` and are refused while they hold data, but a
  `#[flecs(meta)]` struct with a `usize` field that is secretly a handle reflects as a
  clean `uptr` leaf and passes.
- **Anything not registered through the module's own registration.** The manifest is a
  before/after delta around one `register` call. A component some other module registered
  first is attributed to that module, not this one.
- **Non-component state.** Statics and thread-locals inside the module are re-initialised
  by the new dylib; nothing tracks or migrates them.
- **Trailing padding.** Two layouts identical in every leaf but differing in padding
  content compare equal. This is correct — padding is not data — but a type punning through
  padding would break.

### The ABI precondition, made structural by nix

`repr(Rust)` has no stable ABI, so a host and a module built by different compilers can
disagree about the layout of any type they pass, including `String` and `Vec`. Three checks
run before a module's code is trusted, all in `AbiToken`:

- an ABI version integer, bumped when the descriptor types change
- the rustc version, commit hash and host triple, captured by `build.rs` at compile time
- the address of a static in the runtime crate, compared between host and module

The third is the interesting one. Equal addresses prove both sides resolved
`hyperion-hot-reload` to one shared dylib, and therefore share one `flecs_ecs` component
index and one set of flecs C globals. A module built without `-C prefer-dynamic` links its
own static copy, gets its own allocator and its own `ecs_os_api`, and would alias the
host's silently. It is refused with a message naming the fix.

Verified by building the demo module without `-C prefer-dynamic` and loading it:

```
initial load failed: refusing to load module: module linked its own copy of
hyperion-hot-reload (anchor 0x10b78d0be vs host 0x105712f0e). It therefore has its own
flecs component-index counter and its own flecs C globals, which alias the host's
silently. Build the module with `-C prefer-dynamic` so it links the hyperion-hot-reload
dylib rather than its rlib.
```

nix makes the compiler half structural rather than conventional. `flake.nix` now derives
the toolchain from `rust-toolchain.toml` with `fromRustupToolchainFile`, so `nix run
.#hot-reload-demo` cannot build the host and the modules with different compilers. It
previously carried a second pin, `nightly-2025-02-22`, which had already drifted from the
`nightly-2025-05-05` that cargo obeys.

### What the index repo's module system does and does not lend

The brief points at `~/.config/nix/ix/index` as a model. Two things transfer:

- **A declared surface, evaluated before anything is applied.** A NixOS module's `options`
  are a typed declaration separate from the `config` that acts on them, and evaluation fails
  before activation. The manifest is the same shape: a declaration of what a module
  contributes, diffed and accepted or rejected before the live world is touched.
- **`checks` builds every app.** Adopted directly; `nix flake check` building each
  `writeShellApplication` is what proves the scripts pass shellcheck and their tools resolve.

Two things do not:

- **Auto-discovery.** index discovers modules by walking `modules/`. Here, modules are
  explicitly imported through `world.import::<T>()`, and that explicitness is what makes
  the before/after registration delta attributable to one module.
- **Activation as pure recomputation.** A NixOS switch can build the entire new system and
  swap, because the configuration is derived from source. An ECS world holds live mutable
  state that cannot be recomputed, which is the entire reason migrations exist. The one real
  analogue is `system.stateVersion`, and `opaque_versions` is deliberately the same bargain.

## Not done

Stated plainly, because these are the parts a reader cannot see for themselves.

- **No rollback past the tear-down step.** A registration that panics after step 2 leaves
  the module's systems deleted and its components half re-registered. Staging into a second
  world and moving entities across would fix it and was not attempted; it would also change
  the cost model from "touch what changed" to "copy the world".
- **Dylibs leak, by design, and the cost is unbounded.** Address-space growth is
  proportional to the number of reloads. Acceptable for a dev loop; not acceptable for a
  process that reloads indefinitely. There is no way to do better while flecs keeps
  function pointers into module text with no API to unset them.
- **No file watcher.** The demo drives reloads from a list of paths. Wiring this to a
  watcher and to hyperion's tick loop is not done, and the interaction with a mid-tick
  reload is not designed. Reloads must happen between ticks; nothing currently enforces it.
- **Not integrated with hyperion's own modules.** The brief ruled out editing
  `crates/hyperion/` and `events/`, so this runs against a purpose-built demo module. The
  changes those crates would need are: `#[flecs(meta)]` on every reloadable component, and
  an `export_module!` per reloadable module. No change to the runtime crate is required.
- **`nix flake check` does not pass as a whole.** `packages.default` fails with
  `A hash was specified for divan-0.1.17, but there is no corresponding git dependency`.
  This predates this work — it reproduces with these changes stashed — and belongs to the
  flecs migration, whose `cargoLock.outputHashes` still names `flecs_ecs-0.1.3`.
- **Only tested on aarch64-darwin.** `build.rs` has a Linux branch using
  `--export-dynamic` instead of ld64's `-exported_symbol`, and it has never been run.

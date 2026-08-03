# Hot reloading flecs game modules

Edit a game module, rebuild it, and see the change in a running server without
restarting or dropping players. When a component's memory layout changes in a way that
would make the stored bytes mean something different, the reload is **refused** and the
running world is left exactly as it was, unless the developer supplied a migration.

Refusing is the point. The alternative is not a crash; it is a server that keeps running
while every `Health` in the world quietly becomes `2.9e-44`.

- Crate: `crates/hyperion-hot-reload`
- Demo: `nix run .#hot-reload-demo`, source in `crates/hyperion-hot-reload/demo/{host,module}`

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
- **Data-carrying enums — untested.** Only fieldless enums were tried. `Layout::Enum`
  records constant names and order, which for a payload enum would not describe the payload
  at all: a variant's payload type changing at constant size and unchanged variant names
  would compare equal. Whether `#[flecs(meta)]` even accepts a payload enum, and whether one
  reflects as `Enum` or falls through to `Unknown`, was not checked. Until it is, treat a
  payload enum as unprotected and give it an `opaque_versions` entry.
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

nix makes the compiler half structural rather than conventional. `flake.nix` derives the
toolchain from `rust-toolchain.toml` (`rustChannel = (importTOML ./rust-toolchain.toml)
.toolchain.channel`), so `nix run .#hot-reload-demo` builds the host and every module with
one compiler by construction rather than by convention. That single-source pin is not this
work's doing; it landed with the nix apps refactor. It is load-bearing here in a way it is
not elsewhere, because everywhere else a mismatched compiler produces a rebuild, and here
it would produce a module whose `String` has a different layout than the host's.

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
- **`nix flake check` does not pass as a whole.** `packages.default` fails during
  evaluation with `Cargo.lock contains multiple git dependencies with the same
  name-version: valence_*`, because the workspace pins two branches of the same valence
  fork and `cargo-unit` cannot vendor both without losing source identity. This predates
  this work and is unrelated to it: `refactor/flecs`'s own `Cargo.lock` carries the same
  duplicate entries, and this branch's lock differs from it only by the three new crates.
  The two hot-reload checks build and pass on their own:
  `nix build .#checks.<system>.hot-reload-demo .#checks.<system>.hot-reload-registry-guard`.
- **The deployment half is designed, not shipped.** Nothing here is wired to `ix apply`,
  to a systemd unit, or to hyperion's own modules. See "Deploying a reload" below for the
  shape and what is missing, and "Packaging: three derivations" for the part that is now
  measured rather than assumed.

- **The packaged server does not satisfy the shared-pool precondition.** `hyperion` is a
  dylib and the fleet's binary does not link it as one, because `cargoUnit` builds without
  `-C prefer-dynamic`. `checks.hot-reload-index-probe` gates the recipe; nothing yet gates
  the artifact a host runs. Until the packaging lands, loading a module into the deployed
  server is the exact configuration the probe exists to reject.

## Linux, and the one copy of flecs everything depends on

Verified on x86_64-linux (dev-compute-6, rustc 1.99.0-nightly `dc3f85158`). It did not
work there before, in two separate ways, and both failures are quiet enough to be worth
naming.

### Exports: `--export-dynamic` cannot undo what rustc does to a dylib

rustc links a `dylib` with its own anonymous version script ending in `local: *`, which
demotes every symbol it did not generate. flecs's C symbols arrive with `DEFAULT`
visibility and `LOCAL` binding — physically present, dynamically unreachable:

```
$ nm -D --defined-only libhyperion_hot_reload.so | wc -l
9001
$ nm -D --defined-only libhyperion_hot_reload.so | grep -c " ecs_"
0
$ readelf -sW libhyperion_hot_reload.so | grep -w ecs_init
  5905: ... FUNC    LOCAL  DEFAULT   14 ecs_init
```

Neither `-Wl,--export-dynamic` nor `-Wl,--export-dynamic-symbol=ecs_*` helps; a
version-script demotion is not something either flag can reverse. Both were tried and both
left the count at 0. The fix is a *second* version script naming the flecs globs with no
`local:` clause — ld merges version scripts and an explicit pattern beats a `*` wildcard,
so it promotes exactly those and leaves rustc's own exports alone. After: 10717 exported,
666 of them `ecs_*`, `ecs_init` `GLOBAL`.

Note that `-all_load` was never the load-bearing half on macOS either. `flecs_ecs_sys`
compiles `src/flecs_rust.c`, which `#include`s `flecs.c`, so `libflecs.a` has exactly one
member and any reference drags the whole thing in on both platforms. The platforms differ
only in export visibility.

**Where the script goes matters and the wrong placement is silent.** A build script's
`rustc-link-arg` applies to its own crate's artifacts. Putting it in `flecs_ecs_sys` — the
crate flecs's C actually lives in — does nothing, because that crate is an rlib absorbed
into a dylib rather than linked itself. Measured: 0 exported `ecs_*`, no warning. It
belongs in whichever crate *produces the dylib*.

### One pool, or the world is indexed two different ways

`flecs_ecs`'s derive emits, per component type, a `static INDEX` initialised from a
process-global `INDEX_POOL`, and that index is a slot in the world's component array. The
`flecs_manual_registration` note earlier in this document is about the *id* being
per-world; the *index* is not. Two copies of `flecs_ecs` in one process is two pools, and
a module then writes into a slot the host never filled.

Nothing in the reload path detects this. `AbiToken` passes, no error is raised, and both
sides are internally consistent — they simply disagree about which slot is which.

`demo/index-probe-{host,module}` measures it. Two things about how, because the obvious
approaches both give the wrong answer:

- **Do not compare `ecs_init as usize` across the boundary.** An executable taking the
  address of a dynamically-linked function gets its own PLT stub, so the addresses differ
  whether or not the copy is shared. Measured both cases; both printed a mismatch.
- **Do not compare one type's index for equality.** Two independent pools each start at 1,
  so the first type registered on each side reads `1` and `1` and looks shared when nothing
  is. This is what the probe reported before the module referenced the runtime crate at all.

What works is allocation order, which cannot coincide: with one pool, an index taken in the
module is strictly greater than every index the host took first.

With `hyperion` as a plain rlib:

```
host indices: [1, 2, 3, 4] (max 4)
module's own type index: 1
hyperion::simulation::Position index: host 4, module 2
SHARED_POOL=false
```

The *host* is what creates the second copy — it pulls `flecs_ecs` in through hyperion's
rlib while the module resolves it from the runtime dylib. A module that touches no hyperion
component does not avoid this; the host's own linkage is enough.

With `flecs_ecs` and `hyperion` both dylibs:

```
host indices: [1, 2, 3, 4] (max 4)
module's own type index: 5
hyperion::simulation::Position index: host 4, module 4
SHARED_POOL=true
SHARED_HYPERION_INDEX=true
PROBE_OK
```

### A behaviour-only module does not avoid this

There is an appealing argument that it should. `CLAUDE.md` splits every flecs module into a
registration module that only declares components and a behaviour module that only installs
systems and observers, and notes that this lets a consumer import the types without the
systems. Put only behaviour modules in the reloadable library, keep every registration
module in the host, and the manual-registration problem does look like it disappears: a
library that registers nothing cannot collide with anything.

That much is true, and it is the right split for a different reason given below. **It does
not make the shared pool optional.** Registering a component and *looking one up* are
different operations and only the first is avoided. A system's query still resolves `T` to
an id through `T::index()`, so a behaviour module reading `Position` needs the same index
the host filled.

Measured, not argued. `demo/index-probe-module` registers nothing at all — it declares one
marker type and calls `index()` — and in the unshared configuration it read
`hyperion::simulation::Position` as index **2** where the host had it at **4**. A pure
behaviour module querying `Position` would have read a slot the host never wrote.

So the registration/behaviour split is worth keeping, but for the hazard it actually
addresses: **component layout**. A system compiled against one struct layout reading a world
that holds another is silent memory corruption. Keeping component definitions in the host
and having the library depend on them rather than declare them means there is exactly one
definition of each layout in the process. The gate on top of that turns a layout change into
a refusal rather than a corruption.

The honest boundary that falls out, and which belongs in front of anyone using this:
**changing what a system does is a reload; adding or changing a component type is a host
rebuild and a restart.**

### The build recipe

1. `flecs_ecs` needs `crate-type = ["dylib", "rlib"]` and a `build.rs` emitting the version
   script above. Without the dylib you get
   `error: cannot satisfy dependencies so 'flecs_ecs' only shows up once`, because two
   dylibs each bundle their own copy.
2. `hyperion` needs `crate-type = ["dylib", "rlib"]`.

   Both crates now emit two artifacts on every build rather than one -- `flecs_ecs`'s
   dylib is about 37 MB next to a 45 MB rlib. Whether that costs meaningful build time is
   **not established**: a clean `cargo build -p flecs_ecs` measured 8875 ms with both and
   8817 ms with the rlib alone, which is within noise, but a stale dylib in the target
   directory means the second configuration may not have taken effect. Treat the build-time
   cost as unmeasured rather than as shown to be zero, and measure it properly if CI wall
   time matters.
3. Host and every module build with `-C prefer-dynamic`, plus rpaths to the rust sysroot
   and to wherever the dylibs land. On ELF add `-C link-arg=-Wl,--undefined-version`: the
   version script `flecs_ecs`'s build script installs names four globs and both bfd and
   lld treat a pattern matching nothing as an error.

   `nix/hot-reload/packaging.nix` is that recipe, written once, as flags on one
   `cargoUnit` workspace. `checks.hot-reload-index-probe` runs the probe over units out of
   that same workspace, so what the gate measures and what a host runs are the same
   artifacts rather than two builds that agree by construction.

   `-C link-arg=-Wl,--undefined-version` is **not** in the recipe as built. The version
   script `flecs_ecs`'s build script installs still names four globs, and bfd and lld still
   error on a pattern that matches nothing, but the workspace's configured linker does not,
   so nothing needs the flag today. It is left out rather than added defensively: if the
   linker changes, the link fails and names the pattern, which is a better signal than a
   flag nobody can explain.

What makes the pool shared is step 1 and nothing else. It is tempting to think a module has
to *reference* `hyperion-hot-reload` to end up on the shared runtime — an earlier version of
the probe carried a call to `AbiToken::current()` with a comment claiming exactly that.
Removing the dependency entirely leaves the probe passing. The dependency being a dylib is
what shares it; a consumer's import list has nothing to do with it.

**`--allow-shlib-undefined` is gone, and what it stood for is fixed.**
`simulation/metadata/mod.rs` used to hand-write
`impl PartialOrd for $name where $type: PartialOrd`, and for 7 metadata types that bound is
unsatisfiable because glam's `Quat` and `Vec3` have no `PartialOrd`. rustc never codegens
those `partial_cmp` bodies and still lists them in the dylib's export list, so a consumer
needed the flag to link at all. The blanket impl is deleted rather than tolerated: it had
exactly one caller in the whole workspace, `events/bedwars/src/module/regeneration.rs`
comparing two `Health`, and that now compares through `Health`'s own `Deref` to `f32`. So
the flag is not in the recipe, and if it ever needs to come back, that is the signal a
blanket impl came back with it.

**Steps 1 and 2 are landed.** `flecs_ecs` carries the dylib change at
`andrewgazelka/Flecs-Rust` `f09dc53` and `Cargo.toml` pins it; `crates/hyperion` carries
`crate-type = ["rlib", "dylib"]`.

## Deploying a reload

The mechanism a running server needs is not a file watcher. It is systemd's, and NixOS
already exposes it.

`nixos/doc/manual/development/unit-handling.section.md` in nixpkgs: *"If they are different
but only `X-Reload-Triggers` in the `[Unit]` section is changed, **reload** the unit."* So a
game-logic-only change can reach a running server as a `systemctl reload` rather than a
restart, which means the process never exits, the proxy's backend socket never closes, and
no player is disturbed.

Three pieces make that hold:

- `reloadTriggers = [ gameModuleDylib ]`, so the dylib's store path lands in
  `X-Reload-Triggers` **and nowhere else**. If it also appeared in `ExecStart` or
  `Environment` the `[Service]` section would differ and the whole scheme degrades to a
  restart.
- The process reaches the dylib through a stable path — `environment.etc` — since `/etc` is
  rebuilt during activation, before units are acted on, and changing a symlink there is not
  a unit change at all.
- `ExecReload` is a fixed string: a client that asks the running process to reload and
  **exits non-zero when the gate refuses**, printing the refusal and its `migration!` stub.
  A refused reload then surfaces as a failed activation with the reason in the deploy
  output, rather than as a silent no-op, and the world keeps running on the old build.

All three are built. `nix/modules/game-server.nix` is the unit,
`crates/hyperion-reload-client` is the client, and `events/smash`'s `init_game` no longer
calls `app.run()` — see below.

### What replaced `app.run()`, and what had to be reproduced by hand

`App::run` is flecs's `ecs_app_run`, which does not return until the world quits and offers
no per-tick Rust hook. The host therefore calls `world.progress()` itself, which means
everything `ecs_app_run` does to a world *before* its loop has to be done by somebody. Read
out of flecs's own `addons/app.c` at the pinned `flecs_ecs_sys`, in order:

| `ecs_app_run` | where it lives now |
| --- | --- |
| `ecs_set_target_fps(world, desc->target_fps)` | `hyperion::tick_loop::prepare` refuses a world with none |
| `ecs_set_threads(world, desc->threads)` | `HyperionCore`, which already did it |
| `ECS_IMPORT(FlecsRest)` + `ecs_set(EcsWorld, EcsRest, {port})` | `hyperion::tick_loop::prepare` |
| `ECS_IMPORT(FlecsStats)` | `hyperion::tick_loop::prepare` |
| `while (ecs_progress(world, 0)) {}` | `hyperion_hot_reload::service::run` |

Two rows deserve a note, because both look like omissions and neither is.

**Threads.** `HyperionCore` calls `world.set_threads(rayon::current_num_threads())` while it
is being imported. Every event's `init_game` then called `App::set_threads` with the *same
expression*, and flecs's `flecs_set_threads_internal` returns without doing anything when
the stage count already equals the request — so that second call was provably a no-op, and
deleting it changes nothing.

**Target frame rate.** `HyperionCore` sets it to `TICKS_PER_SECOND`. `App::new` read that
same value back out of the world and set it again; what it *also* did was substitute 60
when nothing had set one. `prepare` refuses instead of substituting, because a world with a
target rate of zero does not run slowly — it spins a core per stage as fast as
`ecs_progress` returns, and the only outward symptom is a host that is hot.

The one thing a release gate cannot check here is the flecs registration asserts, which are
compiled out of release builds (CLAUDE.md, ENG-11000). `checks.smash-dev-boot-e2e` boots the
dev-profile binary for that reason, and it covers this loop.

### The unit, as deployed

```ini
[Unit]
X-Reload-Triggers=/nix/store/...-X-Reload-Triggers-hyperion-game-server

[Service]
ExecStart=/nix/store/...-smash-server/bin/smash --ip :: --port 35565 \
  --root-ca-cert ... --cert ... --private-key ... \
  --rules /etc/hyperion/smash-rules.so \
  --reload-socket /run/hyperion-game-server/reload.sock \
  --build-stamp /etc/hyperion
ExecReload=/nix/store/...-hyperion-dylibs/bin/hyperion-reload-client /run/hyperion-game-server/reload.sock
RuntimeDirectory=hyperion-game-server
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
```

Four things in there are load bearing and none of them is obvious:

- **Every path on `ExecStart` is stable.** The rules dylib's store path is in
  `X-Reload-Triggers` and nowhere else — and note that nixpkgs does not inline the triggers,
  it hashes them into a file of their own and names *that*. So "is the dylib in the right
  line" is not a checkable property; "does changing the dylib move exactly one line" is, and
  that is what `checks.hot-reload-unit-split` asserts, by rendering the unit twice.
- **`AF_UNIX` had to be granted.** `nix/modules/common.nix` hardens both services down to
  `AF_INET` and `AF_INET6`. The reload socket is a unix socket, so without this the server
  dies at startup on `Address family not supported by protocol` — on a real host, and
  nowhere else, because nothing in a test or a gate runs under that filter.
- **`ExecReload` ships with the engine, not with the event.** It is part of `[Service]`, so a
  client whose path moved when the rules moved would restart the server on exactly the
  deploys this exists to make invisible. `hyperion-dylibs` moves only on an engine change,
  which restarts anyway.
- **The build stamp is a reload trigger too.** A commit that changes nothing the server
  links still changes `/etc/hyperion/build-rev`, and without a reload the bar would go on
  naming the previous commit — the one question it exists to answer. That reload re-opens a
  byte-identical dylib and cannot be refused, because a schema can only move when the dylib
  does.

### Three things only a running server said, and one gate each

The deployment was built against static evidence -- store paths, `readelf`, `ldd`, rendered
unit files -- and all of it was green. Starting the thing on dev-compute-6 found three
defects in an afternoon, every one of them silent and two of them already shipped.

**The packaged binary segfaulted on startup, in every build ever made of it.** Not under
load: `smash-server/bin/smash --help` exited 139. `#[global_allocator]` and
`-C prefer-dynamic` cannot coexist, because rustc gives each Rust dylib a version script
ending `local: *` -- the same fact this document already records about LMDB -- so each
dylib's `__rust_alloc` is local and uninterposable, and the process runs the system
allocator inside the dylibs and jemalloc inside the binary. The first pointer to cross
takes it out, inside clap, before `main`. Nothing caught it because the fourteen end-to-end
gates boot `gameBinaries.smash`, a `cargoUnit` build with no dylibs at all; the packaged
binary was inspected and never executed. `checks.hot-reload-server-starts` now runs
`--help` on it, which costs milliseconds and needs no certificates, no world and no
network. ENG-12112.

**The reload loaded nothing and said `accepted`.** `dlopen` searches its list of loaded
objects by name before it stats the file, and the loader deliberately never `dlclose`s, so
a second load through `/etc/hyperion/<event>-rules.so` returned the image from the first
and re-ran the old entry point. `MainPID` unchanged, `NRestarts` unchanged, the journal
saying `hot reload accepted`, the client printing `accepted smash-rules bbbbbbb`, the `/etc`
symlink pointing at the new build -- and `/proc/<pid>/maps` naming only the old one. The
trigger is the very thing that makes reload-not-restart work: the path has to be stable,
so the deployed configuration is exactly the one `dlopen` dedupes. `HotReloader::load` now
copies each candidate to a fresh name first. ENG-12113.

**The deployed server logged nothing at all.** `EnvFilter::from_default_env()` with
`RUST_LOG` unset builds a filter with no directives, which passes nothing, and a unit sets
no `RUST_LOG`. So `hot reload accepted` -- the one line that says an invisible deploy
landed -- would never have reached the journal. Measured both ways on the same binary and
unit: zero lines against every line. The default is now `info`, and ANSI is dropped when
stdout is not a terminal, because a service was writing `\x1b[32m INFO\x1b[0m` into the
journal and every severity grep over it matched nothing.

The shape they share is worth more than any of them: **a derivation that is only ever
inspected is not a derivation that is known to work.** Two of the three were introduced by
changes whose evidence sections were entirely static analysis, and static analysis is what
they were correct about.

### Two sibling dylibs may not share an rlib

`crates/hyperion` and `crates/hyperion-hot-reload` are both dylibs and neither depends on
the other, so under `prefer-dynamic` each statically includes its own copy of every rlib it
uses, and a binary linking both is refused:

```
error: cannot satisfy dependencies so `tracing` only shows up once
error: cannot satisfy dependencies so `tracing_core` only shows up once
error: cannot satisfy dependencies so `once_cell` only shows up once
error: cannot satisfy dependencies so `pin_project_lite` only shows up once
```

Those four are `tracing` and its dependencies, and they were the whole list: everything else
`hyperion-hot-reload` uses arrives inside `libflecs_ecs.so`, which is a dylib and therefore
one copy. Three ways out, and only one of them is right here:

- **Make one depend on the other.** Tried and reverted. It fixes the packaged link and
  breaks a plain one -- `cargo test -p hyperion-hot-reload -p smash` then builds
  `libhyperion.so` against `libhyperion_hot_reload.so`, neither with `prefer-dynamic`, and
  duplicates `std` instead.
- **Make the shared crate a dylib.** Not available for a crate we do not own.
- **Stop sharing it.** `hyperion-hot-reload` gave up `tracing` and now returns
  `service::Outcome::{Applied, Refused}` for the host to log. Better layering anyway: a
  library that logs has decided your format, and the severities belong to whoever runs the
  query.

So: before adding a dependency to `hyperion-hot-reload`, check whether `hyperion` has it
too. `cargo tree -p hyperion-hot-reload` intersected with `cargo tree -p hyperion`, minus
whatever `libflecs_ecs.so` already carries, is the list that must stay empty.

### What a reload costs

Measured, debug profile, three runs each. The Linux figures are dev-compute-6 (32 cores);
the smash figure is aarch64-darwin.

| | |
| --- | --- |
| rules-only rebuild and link (`smash`, 9756 lines of rules code, one line edited) | 1.76 / 1.85 / 2.07 s |
| minimal module rebuild and link (165 KB dylib) | 282 / 285 / 283 ms |
| process start **and** `dlopen` — an upper bound on the reload | 31 ms |
| touching the engine instead: host and engine rebuild | 4.42 s |
| unit restart | zero, by construction |
| world rebuild, chunk resend, re-join | zero, by construction |

The last two rows are the point. An engine change costs 2.4x the compile *and* loses the
process, the world and every connected player. A rules change costs neither.

Three things these numbers are not:

- **Not the release profile.** A deployment builds release, which compiles slower. Treat
  these as the shape of the cost, not the deployed figure.
- **Not a rules dylib.** The rules are not a separate crate yet, so the smash row is
  `-p smash` relinking the whole binary. A rules dylib links strictly less, so this
  over-estimates rather than under-estimates.
- **Not smash's reload.** The 31 ms is a 165 KB probe module and includes process startup,
  so the in-process reload is strictly less — but a larger dylib takes longer to `dlopen`,
  and the schema diff scales with component count. The demo separately reports
  `instances rewritten: 0` for a code-only change, which is the case that does no archetype
  moves at all.

For the deployment as a whole, the game server is not the slow part. Most of an `ix apply`
is working out what to deploy rather than deploying it, and that cost is unrelated to
anything here.

## Handing this off: what is left, in order

The mechanism is proven and the deployment is not built. The third step below is the risky
one; the rest are known work.

**Done: make `hyperion` a dylib and settle the build flags.** `crate-type` on
`crates/hyperion`, `-C prefer-dynamic` everywhere, and the ELF-only
`-Wl,--undefined-version`. Wide blast radius -- it changes how every consumer links, and a
plain `cargo build` without those flags builds a host whose pool a module cannot share.
`checks.hot-reload-index-probe` gates it, and the guard was watched failing: dropping
`-C prefer-dynamic` from the recipe reproduces this document's own unshared numbers,
module index 1 against a host that had already taken up to 4.

**1. Split `SmashModule` out of `events/smash` into its own crate, built as a dylib with
`export_module!`.** The rules already avoid the host seam by design, but they reach into
`crate::server`, `crate::flecs_ext` and about fifteen `hyperion::` items, so this is a real
refactor rather than a file move. Registration modules stay in the host per the section
above.

**2. Package it.** No longer the unknown it was; see "Packaging: three derivations, because
two would restart" below, which replaces the guesswork with a measured design.

**Done: wire the NixOS module and the fleet spec.** `reloadTriggers`, the stable `/etc`
path, and an `ExecReload` client that exits non-zero on a refusal, all as designed in
"Deploying a reload" above. `checks.hot-reload-unit-split` renders the unit for two builds
of the rules and asserts that the only line which moved is `X-Reload-Triggers`.

### Do the deployment half first, against a module with nothing in it

The order above is the order the pieces were designed in, and it is the wrong order to
build them in. Reversed:

- **The rules split has no unknowns.** It is nineteen files, 111 `world.component::<T>()`
  calls and 30 system declarations, moved across a crate boundary. Large, mechanical, and
  nothing about it can surprise anyone.
- **The deployment half has all of them.** systemd's reload-versus-restart decision, the
  `/etc` indirection, rpaths that resolve in the store, the `makeWrapper` that used to put
  a per-commit path inside `[Service]`, and whether a title reaches a connected player at
  all. Every one of those is a fact about a running host.

So build the loader, the socket, the `ExecReload` client, the title and the NixOS wiring
against a **trivial** rules module that registers nothing and does one visible thing. That
proves reload-not-restart, the surviving player connection and the title on a dev node
without waiting for the split. Then migrate smash's rules into that module a domain at a
time, each migration a small PR that is already covered by the existing test suite.

The failure this avoids is the expensive one: finishing a nineteen-file refactor and only
then discovering that `[Service]` differs on every apply and nothing ever reloads.

## Packaging: one cargoUnit graph, three sets of store paths

`nix/hot-reload/packaging.nix` builds every hot-reload artifact from one `cargoUnit`
workspace: one derivation per rustc invocation, each unit's source scoped to its own crate
directory. Which store paths move is therefore a fact about the unit graph rather than
something the packaging arranges, and the boundary table below is a description of the
dependency graph rather than a rule anyone has to maintain.

Two flags are the packaging's own, because cargoUnit cannot infer either. `-C
prefer-dynamic`: generating a dylib without it makes rustc statically absorb every
dependency into that dylib, and linking an executable without it makes rustc prefer the
rlib of a crate offering both — either way the server and the rules dylib each get their
own `flecs_ecs`. And an rpath to the toolchain's lib directory, because prefer-dynamic
makes libstd dynamic too. cargoUnit supplies the rpaths for the dylibs inside the graph,
whose store paths only it knows.

Neither flag reaches `-C metadata`: cargoUnit derives that from its own graph identity
hash, not from the rustc arguments it passes. That is what makes the ENG-12053 hazard
class structurally impossible here rather than merely handled — under cargo, changing
nothing but an rpath produced `libflecs_ecs-c1d9502659600761` and
`libflecs_ecs-13cef1f428680a8e`.

### How this design was arrived at, and what it replaced

> **Everything from here to "Checking that one `flecs_ecs` really reached both
> artifacts" is history, kept because the measurements in it are what the design rests
> on. The packaging it describes — three `runCommandCC` derivations each running `cargo
> build` over a tree of hand-written stub crates — no longer exists**, and neither do the
> hazards two of its subsections describe. What is still live and stated above: the
> boundary, the source split, and every reason a multi-output derivation is wrong.

Three things were measured, and together they settled the design.

**`cargoUnit` could not build the module dylib.** Its library support was rlib-only and
said so in an assertion rather than in a comment:

```
# M2: this builder is rlib-only (the filename and extern-path hardcode
# `.rlib`). Reject an artifact that is clearly not an rlib/rmeta so a
# cdylib/staticlib/proc-macro mistake fails loud at eval, not at link.
  Only plain rlib libraries are supported (not cdylib/staticlib/proc-macro).
```

That is fixed (ENG-12078, index#4543): a `dylib` unit now publishes every linkable
artifact it produced and a consumer passes all of them to rustc, which is what cargo does
and what lets rustc pick dynamic linkage. `workspace.libraries.smash_rules` is the route to
`libsmash_rules.so` today.

**The binary the fleet runs today links nothing from the workspace dynamically.** It is a
`cargoUnit` build with no `-C prefer-dynamic`, so `crate-type = ["rlib", "dylib"]` on
`hyperion` changes what is *available* and not what is *linked*:

```
$ otool -L /nix/store/yjlf...-smash-0.1.0/bin/smash
    /System/Library/Frameworks/Security.framework/...
    /System/Library/Frameworks/SystemConfiguration.framework/...
    /System/Library/Frameworks/CoreFoundation.framework/...
    /nix/store/0ky9...-libiconv-115.100.1/lib/libiconv.2.dylib
    /usr/lib/libSystem.B.dylib
$ find /nix/store/yjlf...-smash-0.1.0 -name '*.dylib' -o -name '*.so'
(nothing)
```

Five entries, all system. No `libhyperion`, no `libflecs_ecs`, and no dylib shipped beside
the binary. A module loaded into *that* process gets its own pool, which is the
configuration the probe exists to reject. So the packaged server has to leave the
`cargoUnit` path too, not only the module.

**A multi-output derivation would defeat the whole feature.** The obvious repair — one
cargo build emitting the binary and the dylib as two outputs — is wrong, and quietly so.
Every output of a derivation moves when any input does, so a rules-only edit would move the
binary's store path, `ExecStart` would differ, and systemd would restart. The gate would
pass, the reload would never be attempted, and the only symptom is that players get dropped
on a deploy that should have been invisible.

What the boundary actually requires is that **the module's store path moves when the host's
does not**, and a store path is a function of a derivation's inputs. So the boundary is a
statement about which sources reach which derivation:

| edit | `hyperion-dylibs` | `smash-server` (`ExecStart`) | `smash-rules` (`reloadTriggers`) | deploy |
| --- | --- | --- | --- | --- |
| a rules crate | — | — | moves | **reload** |
| a host crate | — | moves | — | restart |
| an engine crate | moves | moves | moves | restart |

All three link the engine dylibs by rpath into the store, which is what keeps one
`flecs_ecs` in the process. A component's layout lives in the host crate, so changing it
moves `ExecStart` and systemd restarts. A system's body lives in the rules crate, so
changing it moves only the reload trigger and systemd reloads. Nobody has to remember the
rule; it is the dependency graph.

**A host edit does not move the rules dylib, and that is deliberate (ENG-12078).** The
cargo-based packaging moved it, because `mkSource` put the host crate's directory in the
rules derivation's source tree — a consequence of filtering at directory granularity, not
of a dependency edge. `events/smash-rules` depends on `flecs_ecs`, `hyperion-hot-reload`
and `tracing`; it does not depend on `smash`, so cargoUnit correctly rebuilds nothing. A
rules system cannot name a host-owned component type; it reaches components through
`hyperion-hot-reload`'s registry by name, and the loader's layout check is what catches a
component whose shape moved underneath it. That check was always doing this work — on a
host edit the old packaging just also happened to recompile. The deploy is unchanged
either way: a host edit moves `ExecStart`, systemd restarts, and the fresh process loads
and layout-checks the rules dylib, so nothing stale survives.

**The part that will actually cost time is the source filter, and it is worth naming now.**
Each derivation needs a `src` narrow enough that an unrelated commit does not move it, and
wide enough that cargo can resolve the workspace: the root `Cargo.toml`, `Cargo.lock`, and
the member directories in that crate's dependency graph. Get it wrong in the loose
direction and `smash-server` moves on every commit, which is the build-stamp wrapper's bug
wearing a different hat, and every apply restarts. There is a cheap gate for it: build the
three derivations, touch a file in the rules crate, rebuild, and assert that exactly one of
the three paths changed.

#### One `flecs_ecs` needs one package selection, not one derivation

> **Obsolete mechanism, live lesson.** Cargo resolving features per invocation is why the
> old packaging had to pass one identical `-p` selection to three `cargo build`s. There is
> one invocation now, so there is nothing to keep in step. The lesson that survives is what
> the failure looked like, and that `engineUnit` in `nix/hot-reload/packaging.nix` asserts
> exactly one `flecs_ecs` unit exists rather than assuming it.


Splitting the build across three derivations reintroduced the problem the split exists to
prevent. Each derivation runs its own `cargo build`, and building `-p hyperion` in one and
`-p smash-rules` in another put two `flecs_ecs` in one target directory:

```
libflecs_ecs-a576c74c3728f55c.so    from -p hyperion
libflecs_ecs-af57d040ba838c15.so    from -p smash-rules
```

Two `flecs_ecs` is two `INDEX_POOL`s, which is exactly what `checks.hot-reload-index-probe`
rejects. The first guess was that package *selection* and *source filtering* must differ per
derivation by design, so no arrangement of inputs could unify them — that fingerprint
equality across derivations was structurally impossible. That guess was wrong, and
`cargo build --unit-graph` says why in about a minute.

The `flecs_ecs` unit is **byte-identical** under both selections: same 23 features, same
profile, same `crate-types = ["dylib", "rlib"]`. What differs is three of its transitive
dependencies, whose metadata hashes cargo folds into the dependent's:

| crate | `-p hyperion` | `-p smash-rules` |
| --- | --- | --- |
| `bitflags` | `serde`, `serde_core`, `std` | (none) |
| `libc` | `default`, `std` | (none) |
| `syn` | ..., `fold`, `visit` | (fewer) |

`bitflags` is a direct dependency of `flecs_ecs`. Cargo resolves features over the packages
named on the command line — `-p hyperion` reaches 473 units and `-p smash-rules` reaches 75 —
so package selection alone moves the hash.

Which makes the fix cheap and structural rather than clever: **pass the same selection string
to every derivation.** Feature resolution reads manifests and never source, and `mkSource`
already puts every workspace member's `Cargo.toml` into every tree, stubbing only the `.rs`
bodies. So all three invocations resolve over identical inputs by construction. A derivation
whose source stubs a package still compiles that package's dependency graph — which is
exactly the seed the others want — and then compiles an empty `lib.rs` for the package
itself.

The rule that falls out, and the one to keep: the selection may not be narrowed to "the
packages this derivation ships", and may not be a function of the event being built. Either
is the source split written out a second time, in a place where its only symptom is a reload
that silently indexes one world two different ways.

#### The seed may only carry artifacts whose source the consumer agrees with

> **Obsolete mechanism.** There is no seed. The three derivations shared one `target/`
> tarball because three `cargo build`s had to reuse each other's artifacts; cargoUnit
> shares artifacts by making each one its own derivation, so nothing is copied between
> builds and nothing can be stale.


`hyperion-dylibs` tars its target directory so the other two do not recompile the engine, and
they date everything to 2100 because cargo decides freshness by mtime and everything unpacked
from the store shares one normalised timestamp. Once the selection was unified, that seed
also contained a `libsmash.rlib` built from a **stub** — and dated into the future, so the
consuming derivation never rebuilt it from the real source:

```
error[E0432]: unresolved import `smash::init_game`
 --> events/smash/src/main.rs:1:5
1 | use smash::init_game;
  |            no `init_game` in the root
```

`hyperion-dylibs` therefore `cargo clean -p`s every stubbed event member before tarring, with
a guard that fails the build if a stub artifact survives. Cleaning before the copy is also
what keeps a stub `libsmash_rules.so` — same filename as the real rules dylib, none of its
systems — from being shipped beside the engine and landing on every event binary's rpath.

#### An engine dylib hides the C libraries it swallows

`smash-server` failed to link with eight undefined LMDB symbols, and the cause is not where
the error points. `libhyperion.so` contains LMDB's code and hides every byte of it:

```
$ readelf --dyn-syms libhyperion.so | grep -c mdb_
0
$ readelf --syms libhyperion.so | grep mdb_env_open
 14356: 0000000000d0df3c   933 FUNC    LOCAL  DEFAULT   13 mdb_env_open
```

133 definitions, every one `LOCAL`. rustc links a Rust `dylib` with its own anonymous version
script ending in `local: *`, which demotes every symbol arriving from a native static
archive. The discriminator is that `ecs_*` (77) and `flecs_*` (22) *are* exported while
`mdb_`, `AWS_LC` and `deflate` are all at zero — flecs is visible only because `flecs_ecs`
ships a `build.rs` that adds a second version script for precisely this reason.

That is fatal rather than merely wasteful because `heed`'s API is generic: every consumer
monomorphises heed's code into its own rlib and emits its own `mdb_*` calls.
`hyperion-permission` is such a consumer and links into an event's binary statically, while
rustc suppresses lmdb's own `-llmdb` on the grounds that an upstream dylib already provides
it.

`-Wl,--export-dynamic-symbol=mdb_*` does not fix it — measured here leaving the exported
count at zero, independently reproducing what `flecs_ecs/build.rs` documents for `ecs_*`.
A second version script does, and it has to live in the crate that *is* the dylib, because a
build script's `rustc-link-arg` applies to its own crate's artifacts and nothing else. That
is what `crates/hyperion/build.rs` is: 70 exported `mdb_*` after, `mdb_env_open` `GLOBAL`.

This keeps one copy of LMDB in the process. Linking a second `liblmdb.a` into the executable
would also make the link succeed, and would put two copies of a C library with process-global
state in one process — the same shape the index probe exists to reject for flecs. The general
signature, for the next native library that hits this: an undefined `foo_*` at an event's
final link whose definition is `LOCAL` in `libhyperion.so`'s `.symtab`.

### Checking that one `flecs_ecs` really reached both artifacts

`checks.hot-reload-one-flecs` asserts it, on the artifacts that ship. It used to be a manual
`readelf`/`ldd` recipe here, with a note saying to write the check once the cargoUnit
migration made the property structural. It did, so this is that check (ENG-12078).

Two questions, and only asking both is a check:

1. **The same `DT_NEEDED` name.** Two different metadata hashes is two `INDEX_POOL`s: one
   world indexed two different ways, with no crash and no error, components reading as each
   other's neighbours.
2. **The same resolved store path.** The same hash reaching two store paths is the identical
   aliasing fault wearing a nicer name, and a string comparison alone cannot see it.

The build already refuses a dangling `DT_NEEDED` (`requireResolved` in
`nix/hot-reload/packaging.nix`), so each artifact resolves *something*; that is a weaker
property than the two above and does not imply either.

The check fails closed. Each extraction is checked for emptiness before anything is
compared, because "no `flecs_ecs` line found" and "the two `flecs_ecs` lines agree" are
otherwise the same silence. It also asserts the resolved file is the one `hyperion-dylibs`
exposes, so a third copy that happens to be consistent between the two artifacts is still
caught.

What it looks like when it holds:

```console
$ readelf -d .../smash-server/bin/smash            | grep flecs
 0x0000000000000001 (NEEDED)  Shared library: [libflecs_ecs-fa19ab35c63d573f.so]
$ readelf -d .../smash-rules/lib/libsmash_rules.so | grep flecs
 0x0000000000000001 (NEEDED)  Shared library: [libflecs_ecs-fa19ab35c63d573f.so]
$ ldd .../smash-rules/lib/libsmash_rules.so | grep flecs
    libflecs_ecs-fa19ab35c63d573f.so => /nix/store/0hvrp...-flecs_ecs-0.2.2/lib/libflecs_ecs-fa19ab35c63d573f.so
```

Since ENG-12078 the eval-time half is stronger than the runtime half: one cargoUnit graph
resolves features once, so there is one `flecs_ecs` derivation, and `engineUnit` in the
packaging fails the build if the graph ever holds two. This check is what the loader
actually does with the resulting files.

### The boundary, measured rather than asserted

`checks.hot-reload-source-split` asks this directly rather than standing in for it. It
instantiates the packaging over a perturbed source tree and compares `drvPath`s, in both
directions, which is exactly the table above:

```
                 baseline                          after a rules-only edit
hyperion-dylibs  <unchanged>
smash-server     <unchanged>
smash-rules      MOVED

                                                   after a host (component) edit
hyperion-dylibs  <unchanged>
smash-server     MOVED
smash-rules      <unchanged>
```

A `drvPath` that did not move guarantees an `outPath` that did not move, so the assertion
stays conservative in the safe direction even though every unit is content-addressed.

A rules edit moves the reload trigger and leaves `ExecStart` alone, so systemd reloads. A
component edit moves `ExecStart`, so systemd restarts — which is the correct outcome, because
a system compiled against a layout the world no longer holds is memory corruption rather than
a stale build.

Broken once and watched failing, by making the server derivation take the rules unit as an
input:

```
error: hot-reload-source-split: smash-server on a rules edit moved and must not have.
  before: /nix/store/2mpmd2j7v1zy29ha5s0j6ixb9rsp100j-smash-server.drv
  after:  /nix/store/m0sclw0czzabkk2igicalc52pvc6xv62-smash-server.drv
```

### Adopting it costs no scheduled restart

Step 1 changes the host binary, so the running game server has to restart once to pick it
up — but that restart never has to be scheduled *for this*. The fleet already restarts for
version bumps, and the proxy and the game are built from the same repository, so those
move every node anyway. The split host can sit in the tree and take effect on the next
apply that was happening regardless.

Design for that: none of this should want its own window. The claim is then not "one
restart, then none" but that no restart was ever scheduled for it, including the first.

### Reproducing the measurements

Everything in this document was measured on dev-compute-6 (x86_64-linux, 32 cores,
rustc 1.99.0-nightly `dc3f85158`) and on aarch64-darwin, through `nix develop` and cargo.
The build tree under `/tmp/hotreload-elf` on that host **was deleted** when the node was
released, so reproducing means a fresh clone and a warm toolchain fetch — roughly twenty
seconds for the devShell once the store is warm, and a couple of minutes for the first
`cargo build -p hyperion-hot-reload`.

Note that `cargo build -p smash` does **not** work in the devShell on Linux, for reasons
unrelated to any of this: jemalloc 5.3.1's configure cannot survive GCC 15
(`cannot determine return type of strerror_r`). The nix package path is unaffected.
ENG-11279. Until it is fixed, iterate on darwin and use nix for Linux artifacts.

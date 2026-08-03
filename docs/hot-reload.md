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
  shape and what is missing.

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

   `crates/hyperion-hot-reload/index-probe.sh` is that recipe, written once.
   `nix run .#hot-reload-index-probe` runs it, and `checks.hot-reload-index-probe` is the
   same script as a derivation, so the gate and the command a contributor runs by hand
   cannot disagree about the same tree.

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

Not built. `app.run()` in an event's `init_game` is flecs's own main loop and offers no
per-tick Rust hook; it would become an explicit `while world.progress()` so reloads land
between ticks, which is also what the "reloads must happen between ticks" gap above needs.

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

**2. Package it. This is the risky step.** The game server binary and the module dylib have
to be separate store paths, both built with the flags above, with rpaths that resolve in
the nix store rather than in `target/debug`. `checks.hot-reload-index-probe` is the first
nix build of the recipe and it is a `cargo build` inside a sandbox, not a `cargoUnit`
artifact, so it proves the flags and not the packaging. Expect the surprises here.

**3. Wire the NixOS module and the fleet spec.** Designed in "Deploying a reload" above:
`reloadTriggers`, the stable `/etc` path, and an `ExecReload` client that exits non-zero on
a refusal. Small, and the design is settled.

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

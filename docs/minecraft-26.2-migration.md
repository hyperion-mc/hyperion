# Replacing valence and reaching protocol 776

hyperion speaks protocol 763 (Minecraft 1.20.1, June 2023) through a fork of
valence. The current release is 26.2, protocol 776. This document records what
it takes to close that gap without valence, what was built to start, and what
was measured rather than assumed.

Everything below was produced against the real 26.2 server jar
(sha1 `823e2250d24b3ddac457a60c92a6a941943fcd6a`), not from documentation.

## Mojang stopped obfuscating, so the jar is now the best source of truth

This is the fact that changes the economics of the whole migration.

Minecraft 26.1 was the first release shipped unobfuscated, and Mojang stopped
publishing mappings with it. The last version whose metadata carries
`client_mappings`/`server_mappings` is **1.21.11 (2025-12-09)**; the first
without is **26.1-snapshot-1 (2025-12-16)**, and none of the 47 releases since
have them. The 26.2 server jar unpacks to 7,445 classes across 316 packages
with real names throughout — `net/minecraft/network/protocol/game/ClientboundLoginPacket.class`
— and the class files carry `MethodParameters` and `LocalVariableTable` entries
with real identifiers, which even Mojmap never provided.

The practical effect: a decompiler now emits readable Java with correct
parameter names and no mapping step. Recovering a packet layout used to mean
bytecode archaeology against obfuscated symbols; it now means reading source.

The same change broke the tooling everyone else relies on. Fabric detects
unobfuscation by exactly the missing metadata key and substitutes a placeholder
`net.fabricmc:intermediary:0.0.0`, so a Fabric mod asking for `intermediary:26.2`
silently gets an empty mapping. Yarn is over: its Maven `<release>` is
`1.21.11+build.6`, its default branch is `1.21.11`, and it has a commit titled
"Farewell (#4396)" dated 2025-12-11.

## The vanilla generator gives ids and registries, and no layouts at all

`java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --all` needs no
mappings and never did, so it is the one extraction path unaffected by any of
the above. On 26.2 it produces 8,899 files (48 MB) in 1.7 seconds.

What it gives is exactly this, and this is the whole of `packets.json` for one
entry:

```json
"minecraft:intention": {
  "protocol_id": 0
}
```

An id and a name. No fields, no types, no order. That is why every project
needing wire layouts writes its own extractor.

Two other shapes changed in 26.1 and matter to anyone parsing this output:
`reports/items.json` is gone, replaced by 1,537 per-item files under
`reports/minecraft/components/item/`, and `reports/json-rpc-api-schema.json` is
new.

## Layouts are recoverable from decompiled source

Decompiling with cfr takes about ten seconds and produces 676 Java files. Both
codec styles survive it intact.

The manual style reads straight off:

```java
private void write(FriendlyByteBuf output) {
    output.writeFloat(this.health);
    output.writeVarInt(this.food);
    output.writeFloat(this.saturation);
}
```

The composite style keeps its field order, its types, and its combinators:

```java
public static final StreamCodec<ByteBuf, ClientboundResourcePackPushPacket> STREAM_CODEC =
    StreamCodec.composite(
        UUIDUtil.STREAM_CODEC, ClientboundResourcePackPushPacket::id,
        ByteBufCodecs.STRING_UTF8, ClientboundResourcePackPushPacket::url,
        ByteBufCodecs.stringUtf8(40), ClientboundResourcePackPushPacket::hash,
        ByteBufCodecs.BOOL, ClientboundResourcePackPushPacket::required,
        ComponentSerialization.TRUSTED_CONTEXT_FREE_STREAM_CODEC.apply(ByteBufCodecs::optional),
            ClientboundResourcePackPushPacket::prompt,
        ClientboundResourcePackPushPacket::new);
```

A survey of all 271 packet classes in the jar, by codec style:

| style | count | how it is recovered |
| --- | --- | --- |
| manual `write(FriendlyByteBuf)` | 159 | read as a statement sequence; 143 are branch-free |
| `StreamCodec.composite` | 81 | each codec argument evaluated in turn |
| `StreamCodec.unit` (empty) | 14 | trivially, as zero bytes |
| custom `StreamCodec.of` with lambdas | 6 | the encoder's own body, when it is linear |
| abstract bases and inner subclasses | 11 | resolved through the nested class |

The vocabulary is small, which is what makes this tractable at all: 29 distinct
`FriendlyByteBuf` writer methods, 13 `ByteBufCodecs` static fields and 10
combinators cover every packet in the jar.

## Following the codec references, not just naming them, doubles the coverage

An earlier version of the extractor stopped at the first reference it could not
read inline. `Vec3.STREAM_CODEC` became the string `domain:Vec3.STREAM_CODEC`
and the packet was reported partial. On that basis this document previously
concluded that 48% was close to the ceiling and that "better codegen does not
shrink this". **That was wrong, and the measurement that made it look right was
also wrong.** Both are worth spelling out.

The extractor now evaluates a codec expression instead of naming it. Given
`Vec3.STREAM_CODEC` it finds `net.minecraft.world.phys.Vec3`, reads the field's
initializer, sees an anonymous `StreamCodec` whose `encode` body is three
`writeDouble` calls, and returns a struct of three `f64`. The same recursion
handles `StreamCodec.composite`, the `ByteBufCodecs` combinators, inlined
one-line factories with their parameters bound to the caller's arguments,
`StreamCodec.of` over a method reference, and helper methods that take the
buffer as an argument.

Two changes to the surrounding pipeline were needed:

- **The decompile scope had to widen.** Every domain codec a packet reaches
  lives outside `net.minecraft.network`: `BlockPos` in `core`, `Vec3` in
  `world.phys`, the component types in `world.item.component`. The selection is
  computed from the jar rather than listed — `net/minecraft/network`, every
  class whose constant pool names `StreamCodec`, and
  `net/minecraft/core/registries` for the registry key table — which is 1,137 of
  the jar's 7,434 classes, ten seconds, and 676 Java files.
- **The vocabulary shrank rather than grew.** Only the writers that bottom out
  in netty are asserted by hand now; everything in `ByteBufCodecs` is read from
  source, with the hand-written value used only where the derived one comes up
  short. Exactly one entry is still needed, `GAME_PROFILE_PROPERTIES`, whose
  encoder is a loop.

That last change caught a bug in the hand-written table it replaced: it claimed
`ByteBufCodecs.RGB_COLOR` was an `i32` when the source writes three bytes.
Preferring the derived value is what surfaced it.

### Before and after

```
                     before   after
packets                 256     256
fully mechanical        124     202
partial                  77      54
unrecovered              55       0
data components           -     111
  with a known layout     -     100
  simple scalars          -      60
```

**Nine of the 124 were not honest.** The completeness test was
`not wire.startswith("domain:")`, so a domain codec wrapped in a combinator —
`list<domain:KnownPack.STREAM_CODEC>` — passed it. `select_known_packs`,
`command_suggestions`, `move_minecart_along_track`, `recipe_book_add`,
`recipe_book_remove`, `update_attributes`, `set_beacon` and `set_game_rule` were
all reported complete while carrying an unknown element type. The honest
before-figure is **115**, and the honest after-figure is **200**. Completeness
is now a recursive property of the whole type rather than a prefix test on a
string, and the eight recoverable ones are genuinely resolved.

The extractor was re-checked against layouts established independently from the
jar: `ItemLore.STREAM_CODEC` comes out as `list<nbt, max=256>` and
`CustomModelData.STREAM_CODEC` as four successive lists of floats, bools,
strings and ints, both matching. No packet the old extractor called complete
has fewer leaf fields under the new one.

## What is genuinely out of reach, and why

The 56 remaining partial packets and 11 remaining components are not parser
failures. Each one's encoder makes the byte sequence depend on a runtime value:

| cause | packets | example |
| --- | --- | --- |
| `StreamCodec.dispatch` on a runtime type | 10 | particles, recipe displays |
| a branch in the encode body | 14 | `if (itemStack.isEmpty())`, bitfield accumulation |
| id-dispatched payload union | 4 | `custom_payload` in both directions |
| a helper whose own body branches | 12 | chunk data, entity metadata packing |
| a lambda parameter with no static type | 6 | `value.write((FriendlyByteBuf) buffer)` |

`Vec3.LP_STREAM_CODEC` is the clearest case of the second kind. It is a
quantised, variable-length encoding that writes one byte and returns early when
the vector is near zero, so `set_entity_motion` has no fixed layout at all. A
generator that emitted one would be wrong, not incomplete.

These need hand-written codecs. The point of the extractor is that it now says
so precisely, per packet, with the statement that defeated it.

## The data component registry comes out of the pipeline

`ItemStack` is a `VarInt` count, an item registry id, and a patch over the 111
data component types — the largest hand-written surface in the whole protocol
replacement. `DataComponents.java` registers each one as

```java
public static final DataComponentType<Integer> MAX_DAMAGE =
    register("max_damage", b -> b.persistent(ExtraCodecs.POSITIVE_INT)
                                  .networkSynchronized(ByteBufCodecs.VAR_INT));
```

so the name, the registration order and the codec are all readable. Components
registered without `networkSynchronized` are not exempt: `Builder.build()` falls
back to `fromCodecWithRegistries`, which puts the value on the wire as network
NBT of the persistent codec.

Ids are taken from Mojang's `minecraft:data_component_type` registry and
cross-checked against the source registration order, exactly as packet ids are,
and the extractor exits non-zero on disagreement.

The generated `DataComponent` enum carries the id, the resource name, the value
layout and whether that layout is a scalar. 100 of 111 have a layout; 60 of
those are scalars. `crates/hyperion-minecraft-proto/src/item/shape.rs` is the
hand-written table this is meant to replace.

## Layouts are generated now, and only where they are complete

The previous design generated ids, registries and versions, and deliberately no
layouts. That followed from the coverage number, and the coverage number has
moved, so the design follows it.

`generated/wire.rs` carries a `Wire` type and one static per codec the
extractor resolved. `PacketId::layout()` and `DataComponent::layout()` return
`Option<&'static Wire>`. There is no `Unknown` variant and no partial struct:
**a layout that comes back is complete by construction.** The fail-loud rule
that governs the extractor is carried into the generator too, which refuses to
run on a wire kind it has no Rust variant for rather than emitting a placeholder.

```rust
DataComponent::Lore.layout()
// Some(List { element: Nbt, max: Some(256) })

play::clientbound::PacketId::SetHealth.layout()
// Some(Struct([Field { name: "health", wire: F32 },
//              Field { name: "food",   wire: VarInt },
//              Field { name: "saturation", wire: F32 }]))

play::clientbound::PacketId::SetEntityMotion.layout()
// None -- Vec3.LP_STREAM_CODEC is variable-length
```

## The packet structs are generated too, out of a committed protocol.json

The `Wire` table above says what the bytes are. It does not give a Rust type to
put them in, and hand-writing that type for two hundred packets is exactly the
transcription this pipeline exists to delete. So `protocol.json` is committed
into the crate and `build.rs` turns it into the packet structs:

```rust
/// `minecraft:intention`, sent serverbound as handshake id 0.
///
/// Layout from `net.minecraft.network.protocol.handshake.ClientIntentionPacket#STREAM_CODEC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub struct Intention<'a> {
    pub protocol_version: VarInt,
    #[proto(max_len = 32767)]
    pub host_name: &'a str,
    pub port: i16,
    pub intention: ClientIntent,
}
```

**177 of the 180 packet classes the extractor recovered in full are generated.**
The three it declines are named, with the reason, in a comment at the top of the
file they would have been written into:

```
// Layouts the extractor recovered in full but this generator declined, and
// why. Each needs a hand-written codec; none of them is approximated here.
//   minecraft:custom_click_action -- a custom codec inside a combinator needs a hand-written packet
//   minecraft:damage_event -- two fields would both be named `output`
//   minecraft:disguised_chat -- `net.minecraft.network.chat.ChatType$Bound#STREAM_CODEC` contains itself
```

Two hundred packets across 180 classes, because `ClientboundKeepAlivePacket` is
one class that configuration and play both register. It is defined once, in
`packets::common`, and re-exported into each state that sends it, so a value
built for one state is the same Rust value in the other.

### What is committed and what is generated, and why they differ

| artifact | where it lives | why |
| --- | --- | --- |
| `protocol.json` | committed, 660 KB | the input everything else derives from, and the diff a version bump is reviewed as |
| packet structs | `OUT_DIR`, via `build.rs` | the wire contract, where a stale copy silently desynchronises a stream |
| packet ids, registries, versions, `Wire` table | committed `.rs` | data restated as Rust, greppable, and only wrong in a log line |

The line is not "generated versus hand-written", it is **what a mistake costs
and whether the projection is total**. The tables are a total function of
`protocol.json`: every packet gets an id, every registry gets its entries,
nothing is filtered. A committed copy of a total projection can be checked by
diffing it, and `nix flake check` does exactly that.

The packet structs are a *partial* projection — the generator refuses layouts
it cannot express, and the refusal set moves as the extractor improves. That is
precisely the shape that has bitten this pipeline before: an earlier extractor
silently truncated eleven packets and reported them as complete. Making the
filtering happen at build time, from a file whose contents are themselves
guarded, means there is no committed artifact that can disagree with it.

Volume settles the rest. `registry.rs` is 7,700 lines of `&'static str` that
nobody reads top to bottom but everybody greps; moving it into `OUT_DIR` would
make `rg 'minecraft:diamond_sword'` miss, for no gain. The packet structs are
1,300 lines that people *do* read, and `cargo doc` renders them either way.

### The derive

`#[derive(Encode, Decode)]` lives in `crates/hyperion-minecraft-proto-derive`.
A packet body is a fixed sequence of fields with nothing between them, so the
derive is a loop over the fields in declaration order, and anything that is not
that is a compile error rather than a guess.

| attribute | effect |
| --- | --- |
| `#[proto(max_len = N)]` | limit on the innermost string or byte slice |
| `#[proto(max_count = N)]` | limit on the innermost collection's element count |
| `#[proto(with = path)]` | `path::encode` and `path::decode` for this field |

`max_len` and `max_count` thread through `Option<_>` and `Vec<_>` to the type
they constrain, so `#[proto(max_len = 1024)] pages: Vec<&'a str>` bounds each
page rather than the list. Fieldless enums are a `VarInt` discriminant; a
variant with fields is refused, because the discriminant alone does not say how
to read what follows.

The hand-written handshake, status and login codecs are gone: every one of
their packets is generated, and the generated version carries limits the
hand-written one had transcribed by hand.

### Enums are enums

`writeEnum` sends `ordinal()` and `ClientIntent` sends ids of its own, so an
enum field used to come out as a `VarInt`. Both are readable off the jar — the
constants in declaration order, and where the class has one, the `byId` switch
naming each constant's number — so eleven of them are generated as Rust enums:

```rust
/// `net.minecraft.network.protocol.handshake.ClientIntent`, sent as a varint id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum ClientIntent {
    /// `STATUS`
    Status = 1,
    /// `LOGIN`
    Login = 2,
    /// `TRANSFER`
    Transfer = 3,
}
```

The ids are 1, 2, 3 and the ordinals are 0, 1, 2. Reading one for the other
picks the wrong intent silently, which is why the switch is parsed rather than
the position assumed.

## What Pumpkin and the other prior art actually offer

- **Pumpkin-MC/Extractor** is a Kotlin Fabric mod, so it depends on the
  intermediary mappings that no longer exist for 26.x. Its packet output is
  ids and names only — the same information the vanilla generator already
  gives — not field layouts. It is not the tool for this.
- **ViaVersion** is the best-maintained non-jar reference: it registers
  `v26_2 = register(776, "26.2")` and carries real `Protocol1_21_11To26_1` and
  `Protocol26_1To26_2` translation layers whose rewriters encode the actual
  774→775→776 deltas. Worth reading when a layout is ambiguous.
- **PrismarineJS/minecraft-data** has 26.x only on unmerged branches, and
  `pc_26_2` carries just `protocol.json`. Measured against the jar it is
  **incomplete**: 139 play/clientbound and 66 play/serverbound against the real
  141 and 69.
- **valence** is at `PROTOCOL_VERSION = 763` / 1.20.1 — thirteen protocol
  versions behind. **ChunkEdge**, its live fork, is at 770 / 1.21.5 with an open
  issue to reach 26.2. Neither is a path to 776.

## How coupled hyperion is to valence today

`rg -l valence --glob '*.rs'` matches 75 files totalling 17,277 lines, across
eleven valence crates. By crate:

| crate | files touching valence |
| --- | --- |
| `crates/hyperion` | 43 |
| `events/bedwars` | 12 |
| `tools/packet-inspector` | 7 |
| `crates/hyperion-item` | 3 |
| `crates/hyperion-palette` | 2 |
| eight others | 1 each |

The coupling is not uniform, and that matters for sequencing:

- **`valence_protocol` (94 references)** is the real dependency: packet structs,
  `Encode`/`Decode`, and the packet-id constants. Replacing it is the project.
- **`valence_nbt`, `valence_bytes`, `valence_ident`, `valence_text`** are
  small, self-contained format libraries. Each is a few hundred lines to
  reimplement and none is protocol-version-specific.
- **`valence_generated`** is block and item tables — exactly what
  `nix/generate-rust.py` already produces from Mojang's data.
- **`valence_anvil`** is world storage, not protocol, and can stay or be
  replaced independently.
- **`crates/hyperion/src/simulation/metadata/`** (8 files) encodes entity
  metadata serialisers by hand already; it is coupled to valence's types but not
  to its wire code, so it ports rather than gets rewritten.

## Realistic size

Rough, and stated as ranges because the estimate rests on the packet counts
above rather than on having done it:

| piece | estimate |
| --- | --- |
| primitives, framing, compression, encryption | ~1,500 lines — **partly done, see below** |
| NBT (network format, no length prefix) | ~800 lines |
| chat components | ~1,000 lines |
| the 200 mechanical packets | ~2,500 lines, largely transcription |
| `ItemStack` + 111 data component types | ~3,000–5,000 lines, the single biggest item |
| entity metadata (40+ serialiser types) | ~1,500 lines |
| chunk and light data | ~1,200 lines |
| the remaining 56 packets | ~1,500 lines |
| porting the 75 valence-touching files | ~2,000 lines of churn |

Call it 15,000–20,000 lines. The protocol-763-to-776 semantic gap is separate
from all of this and is the part nobody can estimate from the jar: thirteen
versions of behaviour changes, a configuration state that did not exist in
1.20.1, and a new `sessionId` in login.

## What exists now

Built and verified:

- `nix/minecraft-version.json` — the single knob. Version, protocol number, jar
  URL and SRI hash, JDK requirement.
- `nix build .#minecraft-data` — Mojang's generator, sandboxed, 8,899 files.
- `nix build .#minecraft-decompiled` — cfr over the packet and codec classes
  the extractor reads, selected from the jar rather than listed: 698 files.
- `nix build .#minecraft-protocol` — the extraction above, as JSON, including a
  shared table of every named codec it resolved.
- `nix build .#minecraft-proto-rust` — Rust tables: ids, registries, the data
  component registry and the wire layouts.
- `nix run .#update-minecraft-data` — re-resolves Mojang's manifest and rewrites
  the pin, reading the protocol number out of the jar rather than guessing it.
- `nix run .#sync-minecraft-proto` — copies `protocol.json` and the generated
  tables into the crate.
- `nix flake check` — fails if either committed artifact drifts.

The three checks pass:

```
$ nix build --no-link .#checks.aarch64-darwin.minecraft-proto-generated \
                      .#checks.aarch64-darwin.minecraft-proto-json \
                      .#checks.aarch64-darwin.minecraft-protocol
$ echo $?
0
```

The `protocol.json` guard was watched failing before it was trusted. Editing
the committed copy's protocol number to 999 and re-running gives:

```
committed protocol.json is stale; run: nix run .#sync-minecraft-proto
@@ -1,7 +1,7 @@
  {
    "version": {
      "id": "26.2",
 -    "protocolVersion": 999,
 +    "protocolVersion": 776,
```

`nix flake check` as a whole still fails, on a defect that predates this work
and is unrelated to it: `Cargo.lock` carries nine valence crates from two git
sources at the same name-version, and cargo-unit refuses to build an aggregate
vendor directory for that. `packages.bedwars` will not evaluate. No Cargo file
was touched here.

Every derivation rebuilds bit-identically:

```
$ nix build --rebuild .#minecraft-decompiled .#minecraft-protocol \
                      .#minecraft-proto-rust .#minecraft-data
checking outputs of '/nix/store/...-minecraft-decompiled-26.2.drv'...
checking outputs of '/nix/store/...-minecraft-generated-data-26.2.drv'...
checking outputs of '/nix/store/...-minecraft-protocol-26.2.json.drv'...
checking outputs of '/nix/store/...-hyperion-minecraft-proto-generated-26.2.drv'...
$ echo $?
0
```

**No output normalisation was needed**: the vanilla generator sorts its output
and the jar's entries are stamped 1980-02-01, so it is reproducible as-is.

The updater regenerates the pin byte-identically:

```
$ cp nix/minecraft-version.json /tmp/pin-before.json
$ nix run .#update-minecraft-data -- 26.2
$ diff /tmp/pin-before.json nix/minecraft-version.json && echo identical
identical
```

In `crates/hyperion-minecraft-proto`: generated tables for protocol 776, 256
packet ids, 95 registries (6,979 entries), the 111 data component types, and
the wire layouts of the 200 packets and 100 components the extractor resolved.
Alongside them, generated structs for 177 packet classes, and hand-written
codecs for NBT, text components and item stacks.

`crates/hyperion-minecraft-proto-derive` supplies `#[derive(Encode, Decode)]`.

## What the nix audit found

Four things were wrong, all of the same shape: something the pipeline depended
on was not declared.

- **The jar was the one unfree file the unfree policy did not look at.** The
  flake narrows `allowUnfreePredicate` to derivations whose name starts with
  `minecraft-`, but `fetchurl` named the jar `server.jar` and gave it no
  licence, so the gate that exists for Mojang's EULA never saw it. Renamed to
  `minecraft-server-${version}.jar` with `license = unfree`, which the
  predicate now matches.
- **`update-minecraft-data` and `sync-minecraft-proto` called `git`, `cp`,
  `find` and `unzip` without declaring them.** `writeShellApplication` only
  prepends `runtimeInputs` to `PATH`, so those calls resolved against whatever
  the caller happened to have installed. Both now declare what they use.
- **The decompile pulled in a JDK it did not need.** nixpkgs already wraps cfr
  with a runtime. Removing it leaves the output byte-identical, checked with
  `diff -rq` between the two store paths.
- **An inner-class loop ended in `[ -e "$f" ] && printf ...`**, whose non-zero
  status on the last iteration becomes the loop's and would trip `pipefail`.
  It worked by luck of which class happened to sort last. Now an `if`.

Two things were checked and left alone. `flake.nix` is untouched, so
`minecraftPkgs` still narrows unfree to that one prefix rather than flipping
`allowUnfree`. And the generated Rust is *not* marked unfree, even though it is
derived from the jar: it is names and numbers rather than expression, it is
committed under the repo's own licence, and marking it would make the crate
unbuildable. That is a judgement, not an oversight, so it is written down here.

The scripts remain stdlib-only Python under `writers.writePython3Bin`; no
third-party dependency was needed and so no uv project was introduced. Three
flake8 checks are disabled, each with the reason in `nix/minecraft-data.nix`:
`E501` for the format strings that mirror the Rust they emit, and `E203` and
`W503` because both contradict what PEP 8 and every current formatter do.

## What was not verified

Stated plainly, because these are the parts a reader cannot check by looking:

- **Layouts were validated by reading, not by round-tripping.** Only the
  handshake, status and login packets are exercised against a real server. The
  202 layouts the extractor now emits were checked three ways — against the
  independently-established `ItemLore` and `CustomModelData` shapes, against
  the previous extractor's output for the packets both call complete, and by
  reading a dozen of the largest by hand — but no play packet has been sent.
- **Field limits come from the writer, and the server enforces them on the
  reader.** The two disagree more often than they look like they should:
  `ClientIntentionPacket` writes `hostName` with the default 32767 and reads it
  with 255, and `ClientboundHelloPacket` writes `serverId` unbounded and reads
  it with 20. The generated struct follows the writer, so it is permissive
  where vanilla is strict — a decoder that accepts a 300-character host the
  server would reject. It cannot desynchronise a stream, but it is a real
  divergence and the fix is to parse the private `(FriendlyByteBuf)`
  constructor as a second, independent statement of the layout and cross-check
  it against the writer, the way packet ids are already cross-checked.
- **`port` is written as `i16` and read as `u16`.** Same two bytes, same cause
  as above: `writeShort` against `readUnsignedShort`. The generated struct says
  `i16`. Any other field with asymmetric signedness is misreported the same
  way, and the same cross-check would find them all.
- **Three complete layouts are declined rather than generated**, listed in the
  file each would have gone into. `damage_event` is the one that is a defect
  rather than a limit: the extractor inlines `writeOptionalEntityId(output,
  this.sourceCauseId)` without binding the helper's parameter back to the
  caller's argument, so two fields are both labelled `output`. Binding it would
  recover the packet.
- **The extractor being loud is a property of its structure, not a proof.**
  Every statement of an encode body and every argument of a composite has to be
  accounted for, and anything unmodelled propagates up as unresolved. That is
  checked by construction rather than by a test that deliberately breaks it.
- **`GAME_PROFILE_PROPERTIES` is the one asserted layout.** It was transcribed
  from the 26.2 source by hand because its encoder is a loop. Everything else in
  the vocabulary either bottoms out in a netty write or is derived.
- **The pipeline was only run on aarch64-darwin.** The derivations are
  system-agnostic but no other platform was built.
- **The 26.x behavioural deltas were not investigated.** Reports of a new
  `sessionId` UUID in login (which the extractor does show), an `online_mode`
  boolean in `ClientboundLogin`, and entity id 0 becoming a sentinel are
  unconfirmed against the jar.
- **The valence line count is a proxy.** 17,277 lines is the size of files
  mentioning valence, not the size of the change; most of those files touch it
  in a handful of places.

## Notes for whoever picks this up

The extractor is deliberately loud, and it has now been wrong in both available
directions, which is the most useful thing in this document.

It first went wrong by dropping what it could not model. Meeting
`this.payload.write(output)` in the login custom-query packets, it emitted the
fields it had understood and moved on: 135 mechanical packets, of which 11 were
silently truncated layouts. Failing the whole packet instead dropped the count
to 124.

It then went wrong by testing completeness with `not wire.startswith("domain:")`.
That reads a string, and a domain codec wrapped in a combinator does not start
with `domain:` — `list<domain:KnownPack.STREAM_CODEC>` passed. Nine packets were
reported complete while carrying an element type nobody knew. The honest figure
was 115.

Both bugs look like the opposite of each other and are the same bug: a
completeness answer produced by something other than walking the whole type.
Completeness is now recursive over the type and its named references, and
`layout()` returns `Option`, so a caller cannot receive a partial answer at all.
Keep both properties.

Packet ids are cross-checked between two independent sources — Mojang's
`packets.json` and the registration order recovered from the decompiled
`*Protocols` classes — and the extractor exits non-zero on any disagreement. It
caught a real off-by-one during development: `withBundlePacket` registers the
bundle delimiter at play/clientbound id 0 without going through `addPacket`. The
data component ids are cross-checked the same way, against
`minecraft:data_component_type` and `DataComponents`' own registration order.

The hand-modelled vocabulary is checked against the source before extraction
starts. Every `FriendlyByteBuf` writer and `ByteBufCodecs` field the tables name
must still exist in the decompiled jar, or the build fails. That is what turns a
Mojang rename from a field that quietly stops being modelled into an error.

The Python is stdlib-only, so it is packaged with `writers.writePython3Bin`. If
either script ever needs a third-party dependency, the repo convention is a uv
project with `buildUvApplication` rather than adding libraries to the writer.

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

## Layouts are recoverable from decompiled source, for about half the packets

Decompiling `net.minecraft.network` with cfr takes 3.5 seconds and produces 411
Java files. Both codec styles survive it intact.

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

| style | count | recoverable by a shallow parser |
| --- | --- | --- |
| manual `write(FriendlyByteBuf)` | 159 | 143 are branch-free |
| `StreamCodec.composite` | 81 | field order and codec refs, yes |
| `StreamCodec.unit` (empty) | 14 | trivially |
| custom `StreamCodec.of` with lambdas | 6 | no |
| abstract bases and inner subclasses | 11 | n/a |

The vocabulary is small: 29 distinct `FriendlyByteBuf` writer methods, 13
`ByteBufCodecs` static fields, and 10 `ByteBufCodecs` combinators cover
everything.

What the extractor in `nix/extract-protocol.py` actually achieves on 26.2:

```
protocol 776 (26.2)
  packets:            256
  fully mechanical:   124
  partial:            77
  unrecovered:        55
  domain codecs:      53
  registries:         95
  id cross-check:     ok
```

**48% of packets are fully mechanical. That number does not get much better,
and the reason is not parser quality.**

## The remaining work is domain codecs, not packet parsing

The 132 packets that are not fully mechanical bottom out in 53 distinct
domain-type codecs, each with its own transitive closure:

- `ComponentSerialization` (13 packets) — the whole chat component tree, NBT-encoded after login and JSON before it.
- `ItemStack.OPTIONAL_STREAM_CODEC` (7 packets) — pulls in the entire data-component system, 111 component types in the `minecraft:data_component_type` registry.
- entity metadata, chunk sections, particle types, recipe displays, waypoints.

Writing a perfect packet-structure generator would still leave every one of
these to implement by hand, because they are not packets and their codecs are
scattered across the 7,000-class jar rather than concentrated in
`net.minecraft.network.protocol`. **Better codegen does not shrink this; it is
most of the work.**

So: **fully automatic packet-struct generation is not the right target.** The
design this repo now uses is a hybrid — generate what Mojang states as data
(ids, registries, versions), and hand-write codecs against decompiled source,
with the extractor's JSON acting as a checklist and a diff tool across version
bumps rather than as a code generator.

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
| the 124 mechanical packets | ~2,500 lines, largely transcription |
| `ItemStack` + 111 data component types | ~3,000–5,000 lines, the single biggest item |
| entity metadata (40+ serialiser types) | ~1,500 lines |
| chunk and light data | ~1,200 lines |
| the remaining 132 packets | ~3,000 lines |
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
- `nix build .#minecraft-decompiled` — cfr over `net.minecraft.network`, 411 files.
- `nix build .#minecraft-protocol` — the extraction above, as JSON.
- `nix build .#minecraft-proto-rust` — Rust tables.
- `nix run .#update-minecraft-data` — re-resolves Mojang's manifest and rewrites
  the pin, reading the protocol number out of the jar rather than guessing it.
- `nix run .#sync-minecraft-proto` — copies generated Rust into the crate.
- `nix flake check` — fails if the committed generated sources drift.

Both checks added here pass:

```
$ nix build --no-link .#checks.aarch64-darwin.minecraft-proto-generated \
                     .#checks.aarch64-darwin.minecraft-protocol
$ echo $?
0
```

`nix flake check` as a whole still fails, on a defect that predates this work:
`flake.nix` declares a `cargoLock.outputHashes` entry for `divan-0.1.17` while
`Cargo.lock` resolves divan to `0.1.21`, so `packages.default` will not
evaluate. That mismatch is present at commit `313503c`, before any change here,
and no Cargo file was touched by this work. It is left alone deliberately: the
ECS revert in flight will regenerate `Cargo.lock` and change divan's resolution
again, so fixing it now would only conflict.

All five derivations rebuild bit-identically under `nix build --rebuild`. **No
output normalisation was needed**: the vanilla generator sorts its output and
the jar's entries are stamped 1980-02-01, so it is reproducible as-is.

In `crates/hyperion-minecraft-proto`: generated tables for protocol 776, 256
packet ids and 95 registries (6,979 entries), plus hand-written codecs for
handshake, status and login. 18 wire tests, and a live test that completes a
status ping against a real 26.2 server.

Nothing in `crates/hyperion/src/net/` was touched. The new crate is additive and
nothing depends on it yet.

## What was not verified

Stated plainly, because these are the parts a reader cannot check by looking:

- **The extractor's field types were spot-checked, not validated.** Only the 13
  handshake/status/login packets were round-tripped. The other 243 entries in
  `protocol.json` are unproven, and the `list<...>` and `option<...>`
  annotations on higher-order writers are marked `unresolved` precisely because
  the element type was not chased through the lambda.
- **`port` is reported as `i16` and implemented as `u16`.** The server writes it
  with `writeShort` and reads it with `readUnsignedShort`; the extractor's
  vocabulary table follows the writer, the Rust follows the reader. Same two
  bytes, but the mismatch is real and any other field with asymmetric
  signedness would be misreported the same way.
- **Only `UUIDUtil.STREAM_CODEC` was hand-verified** among the 54 domain codecs.
  The other 53 are reported as `domain:` and deliberately not resolved.
- **The pipeline was only run on aarch64-darwin.** The derivations are
  system-agnostic but no other platform was built.
- **No login, configuration or play packet was exercised against a real server.**
  The live test covers the status path only, which never leaves plaintext and
  never enters the configuration state. Compression and encryption are
  unimplemented and untested.
- **The 26.x behavioural deltas were not investigated.** Reports of a new
  `sessionId` UUID in login (which the extractor does show), an `online_mode`
  boolean in `ClientboundLogin`, and entity id 0 becoming a sentinel are
  unconfirmed against the jar.
- **The valence line count is a proxy.** 17,277 lines is the size of files
  mentioning valence, not the size of the change; most of those files touch it
  in a handful of places.

## Notes for whoever picks this up

The extractor is deliberately loud. When it meets a `write` method containing a
statement it does not model — `this.payload.write(output)` in the login
custom-query packets — it reports the packet as unrecovered rather than
emitting the fields it did understand. An earlier version did the latter and
reported 135 mechanical packets instead of 124; the extra 11 were silently
truncated layouts, which is worse than a gap because it looks like an answer.
Keep that property.

Packet ids are cross-checked between two independent sources — Mojang's
`packets.json` and the registration order recovered from the decompiled
`*Protocols` classes — and the extractor exits non-zero on any disagreement. It
caught a real off-by-one during development: `withBundlePacket` registers the
bundle delimiter at play/clientbound id 0 without going through `addPacket`.

The Python is stdlib-only, so it is packaged with `writers.writePython3Bin`. If
either script ever needs a third-party dependency, the repo convention is a uv
project with `buildUvApplication` rather than adding libraries to the writer.

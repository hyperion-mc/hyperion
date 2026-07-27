# hyperion-minecraft-proto

The Minecraft Java Edition wire protocol, generated from Mojang's own data. No
third-party protocol crate.

Currently targets **Minecraft 26.2, protocol 776**.

## Where each thing comes from

`nix/minecraft-data.nix` runs Mojang's data generator and cfr over the server
jar, and `nix/extract-protocol.py` follows the server's own stream codecs down
to the bytes. Its output is `protocol.json`, committed here. Everything else is
derived from that one file.

| artifact | where it lives | how it gets there |
| --- | --- | --- |
| `protocol.json` | committed | `nix run .#sync-minecraft-proto` |
| `packets::` | `OUT_DIR`, at build time | `build.rs` |
| `generated::{packet_id, registry, version, wire, data_component}` | committed | `nix run .#sync-minecraft-proto` |
| `types::` primitives, `nbt`, `text`, `item` | hand-written | against the decompiled sources |

`nix flake check` fails if either committed artifact drifts from what the
pipeline produces, which is what makes the committed copies trustworthy rather
than merely present.

The split is deliberate and `docs/minecraft-26.2-migration.md` argues it: the
tables are a *total* projection of `protocol.json` and are what a reader greps,
so they are committed; the packet structs are a *partial* one — the generator
declines what it cannot express — so they are produced at build time, where no
committed copy can disagree with the filter.

## What a generated packet looks like

```rust
use hyperion_minecraft_proto::{PROTOCOL_VERSION, packets::handshake, types::ClientIntent};

let hello = handshake::serverbound::Intention {
    protocol_version: PROTOCOL_VERSION,
    host_name: "localhost",
    port: 25565,
    intention: ClientIntent::Login,
};
```

A number is a number. How many bytes it costs on the wire is the wire's
business, so the struct behind that call site is

```rust,ignore
pub struct Intention<'a> {
    #[proto(varint)]
    pub protocol_version: i32,
    #[proto(max_len = 32767)]
    pub host_name: &'a str,
    pub port: i16,
    pub intention: ClientIntent,
}
```

and `VarInt` never appears where a caller has to type it. A type still appears
where it means something the wire distinguishes and Rust does not: `BlockPos`
is eight bytes with three coordinates in them, and `RegistryId` is an index
into a named registry rather than a count.

177 of the 180 packet classes whose layout the extractor recovered in full are
generated. **A layout it could not follow all the way has no struct at all**, so
a generated codec is complete by construction; the three it declines are named,
with the reason, at the top of the file they would have been written into.

Names are Mojang's own. Since 26.1 the server ships unobfuscated, so every
symbol here greps directly against the server jar.

## State

Every state has generated packets. Alongside them, hand-written codecs for the
54 packets whose encoder branches on a runtime value -- `ClientInformation`,
`CustomPayload` and `UpdateTags` among them -- plus NBT, text components and
item stacks. Not started: chunk data, entity metadata, compression, encryption.

Eleven packets are currently defined twice, once hand-written and once
generated. `src/packets/mod.rs` says which, and why the reconciliation is a
separate change.

## Regenerating


```sh
nix run .#update-minecraft-data      # re-resolve Mojang's manifest, rewrite the pin
nix run .#sync-minecraft-proto       # rewrite protocol.json and src/generated/
```

## Testing against a real server

The unit tests need nothing. The live test needs a server:

```sh
HYPERION_MC_SERVER=127.0.0.1:25565 cargo test -p hyperion-minecraft-proto \
    --test live_server -- --ignored --nocapture
```

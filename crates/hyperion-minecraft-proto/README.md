# hyperion-minecraft-proto

The Minecraft Java Edition wire protocol, generated from Mojang's own data. No
third-party protocol crate.

Currently targets **Minecraft 26.2, protocol 776**.

## What is generated and what is not

`src/generated/` comes out of the nix pipeline in `nix/minecraft-data.nix` and
is committed so `cargo build` works without nix. `nix flake check` fails if the
committed copy drifts from what the pipeline produces.

| module | contents | source |
| --- | --- | --- |
| `generated::version` | protocol 776, game version, world version | the jar's own `version.json` |
| `generated::packet_id` | 256 packet ids across five states | Mojang's data generator, cross-checked against the server's registration order |
| `generated::registry` | 95 registries, 6,979 entries | Mojang's data generator |

`packets::` is **not** generated. Packet layouts live in `StreamCodec`
compositions in the server's code, and about half of them bottom out in domain
codecs — `ItemStack`, chat components, entity metadata — that no shallow
analysis recovers. Codecs are written by hand against the decompiled sources,
with the extractor's `protocol.json` acting as a checklist. See
`docs/minecraft-26.2-migration.md` for the measurements behind that decision.

Names are Mojang's own. Since 26.1 the server ships unobfuscated, so every
symbol here greps directly against the server jar.

## State

Implemented: handshake, status, login. Configuration and play are not started.
Compression and encryption are not implemented.

## Regenerating

```sh
nix run .#update-minecraft-data      # re-resolve Mojang's manifest, rewrite the pin
nix run .#sync-minecraft-proto       # regenerate src/generated/
```

## Testing against a real server

The unit tests need nothing. The live test needs a server:

```sh
HYPERION_MC_SERVER=127.0.0.1:25565 cargo test -p hyperion-minecraft-proto \
    --test live_server -- --ignored --nocapture
```

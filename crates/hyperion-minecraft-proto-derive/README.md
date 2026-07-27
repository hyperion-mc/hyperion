# hyperion-minecraft-proto-derive

`#[derive(Encode, Decode)]` for the Minecraft wire format.

A packet body is a fixed sequence of fields with nothing between them, so the
derive is a loop over the fields in declaration order. Anything that is not
that — a length that is not a `VarInt`, a discriminant that selects a layout —
is deliberately out of scope: a derive that guessed would produce a codec that
looks right and desynchronises the stream.

```rust,ignore
use hyperion_minecraft_proto::{Decode, Encode, Uuid};

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Hello<'a> {
    #[proto(max_len = 16)]
    pub name: &'a str,
    pub profile_id: Uuid,
}
```

## What it supports

- Named, tuple and unit structs. A unit struct occupies no bytes, matching the
  server's `StreamCodec.unit` packets.
- Fieldless enums, written as a `VarInt` discriminant. An unrecognised value
  decodes to `Error::InvalidEnum` rather than to a wrong variant.
- Borrowed fields: a type with one lifetime gets `impl<'a> Decode<'a>`, one
  with none gets a fresh lifetime.
- Type parameters, bounded with `Encode` and `Decode<'de>` respectively.

## Field attributes

| attribute | effect |
| --- | --- |
| `#[proto(max_len = N)]` | limit on the innermost string, in UTF-16 code units |
| `#[proto(max_count = N)]` | limit on the innermost collection's element count |
| `#[proto(with = path)]` | use `path::encode` and `path::decode` for this field |

`max_len` and `max_count` thread through `Option<_>` and `Vec<_>` to the type
they constrain, so `#[proto(max_len = 1024)] pages: Vec<&'a str>` bounds each
page rather than the list.

## What it refuses

Every one of these is a compile error rather than a silent approximation:

- an enum variant with fields, since the discriminant alone does not say how to
  read what follows
- more than one lifetime, since the derive would have to guess which one the
  reader lends to
- a `with` beside a limit, since the limit would never be applied
- a union

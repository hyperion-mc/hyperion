//! `Encode` and `Decode` derives for the Minecraft wire format.
//!
//! A packet body is a fixed sequence of fields with nothing between them, so
//! the derive is a loop over the fields in declaration order. Everything that
//! is not that -- a length that is not a `VarInt`, a discriminant that selects
//! a layout -- is out of scope on purpose: a derive that guessed would produce
//! a codec that looks right and desynchronises the stream.
//!
//! # Structs
//!
//! ```ignore
//! #[derive(Encode, Decode)]
//! pub struct Hello<'a> {
//!     #[proto(max_len = 16)]
//!     pub name: &'a str,
//!     pub profile_id: Uuid,
//! }
//! ```
//!
//! Fields are written in order and read in the same order. Named, tuple and
//! unit structs all work; a unit struct occupies no bytes, which is how the
//! server's own `StreamCodec.unit` packets are shaped.
//!
//! # Enums
//!
//! ```ignore
//! #[derive(Encode, Decode)]
//! #[repr(i32)]
//! pub enum ClientIntent {
//!     Status = 1,
//!     Login = 2,
//!     Transfer = 3,
//! }
//! ```
//!
//! Every variant must be fieldless. The discriminant is written as a `VarInt`,
//! and a value naming no variant decodes to [`Error::InvalidEnum`] rather than
//! to a silently wrong variant.
//!
//! # Field attributes
//!
//! | attribute | effect |
//! | --- | --- |
//! | `#[proto(max_len = N)]` | limit on the innermost string, in UTF-16 code units |
//! | `#[proto(max_count = N)]` | limit on the innermost collection's element count |
//! | `#[proto(with = path)]` | use `path::encode` and `path::decode` for this field |
//!
//! `max_len` and `max_count` thread through `Option<_>` and `Vec<_>` to the
//! type they constrain, so `#[proto(max_len = 1024)] pages: Vec<&'a str>`
//! bounds each page rather than the list. Both limits are enforced on write as
//! well as on read: the server checks on read only, but a value that cannot be
//! read is one that should never have been sent.
//!
//! [`Error::InvalidEnum`]: ../hyperion_minecraft_proto/enum.Error.html

mod attr;
mod expand;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derive [`Encode`](../hyperion_minecraft_proto/trait.Encode.html).
#[proc_macro_derive(Encode, attributes(proto))]
pub fn derive_encode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand::encode(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive [`Decode`](../hyperion_minecraft_proto/trait.Decode.html).
#[proc_macro_derive(Decode, attributes(proto))]
pub fn derive_decode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand::decode(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

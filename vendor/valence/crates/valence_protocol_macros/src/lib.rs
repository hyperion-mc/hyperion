#![doc = include_str!("../README.md")]
#![deny(
    rustdoc::broken_intra_doc_links,
    rustdoc::private_intra_doc_links,
    rustdoc::missing_crate_level_docs,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_rust_codeblocks,
    rustdoc::bare_urls,
    rustdoc::invalid_html_tags
)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_lifetimes,
    unused_import_braces,
    unreachable_pub,
    clippy::dbg_macro
)]

use proc_macro::TokenStream as StdTokenStream;
use proc_macro2::TokenStream;
use syn::{Attribute, GenericParam, Generics, LitInt, Result, Variant, parse_quote};

mod decode;
mod encode;
mod packet;

#[proc_macro_derive(Encode, attributes(packet))]
pub fn derive_encode(item: StdTokenStream) -> StdTokenStream {
    match encode::derive_encode(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

#[proc_macro_derive(Decode, attributes(packet))]
pub fn derive_decode(item: StdTokenStream) -> StdTokenStream {
    match decode::derive_decode(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

#[proc_macro_derive(DecodeBytes, attributes(packet))]
pub fn derive_decode_bytes(item: StdTokenStream) -> StdTokenStream {
    match decode::derive_decode_bytes(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

#[proc_macro_derive(DecodeBytesAuto, attributes(packet))]
pub fn derive_decode_bytes_auto(item: StdTokenStream) -> StdTokenStream {
    match decode::derive_decode_bytes_auto(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

#[proc_macro_derive(Packet, attributes(packet))]
pub fn derive_packet(item: StdTokenStream) -> StdTokenStream {
    match packet::derive_packet(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

fn pair_variants_with_discriminants(
    variants: impl IntoIterator<Item = Variant>,
) -> Result<Vec<(i32, Variant)>> {
    let mut discriminant = 0;
    variants
        .into_iter()
        .map(|v| {
            if let Some(i) = parse_tag_attr(&v.attrs)? {
                discriminant = i;
            }

            let pair = (discriminant, v);
            discriminant += 1;
            Ok(pair)
        })
        .collect::<Result<_>>()
}

fn parse_tag_attr(attrs: &[Attribute]) -> Result<Option<i32>> {
    for attr in attrs {
        if attr.path().is_ident("packet") {
            let mut res = 0;

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("tag") {
                    res = meta.value()?.parse::<LitInt>()?.base10_parse::<i32>()?;
                    Ok(())
                } else {
                    Err(meta.error("unrecognized argument"))
                }
            })?;

            return Ok(Some(res));
        }
    }

    Ok(None)
}

fn add_trait_bounds(generics: &mut Generics, trait_: TokenStream) {
    for param in &mut generics.params {
        if let GenericParam::Type(type_param) = param {
            type_param.bounds.push(parse_quote!(#trait_))
        }
    }
}

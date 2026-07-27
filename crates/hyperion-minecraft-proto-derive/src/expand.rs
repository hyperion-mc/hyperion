//! Turning a type definition into `Encode` and `Decode` impls.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Fields, GenericParam, Generics, Index, Lifetime,
    LifetimeParam, Result, Type, TypeParamBound, parse_quote, spanned::Spanned,
};

use crate::attr;

/// Path to the runtime crate. `extern crate self as hyperion_minecraft_proto`
/// in the crate root is what makes this resolve from inside it as well.
fn krate() -> TokenStream {
    quote!(::hyperion_minecraft_proto)
}

pub fn encode(input: &DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let krate = krate();

    let body = match &input.data {
        Data::Struct(data) => encode_struct(data)?,
        Data::Enum(data) => encode_enum(data)?,
        Data::Union(_) => return Err(unsupported(input, "a union has no wire form")),
    };

    let mut generics = input.generics.clone();
    bound_type_params(&mut generics, parse_quote!(#krate::Encode));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Encode for #name #ty_generics #where_clause {
            fn encode(&self, writer: &mut #krate::Writer) -> #krate::Result<()> {
                #body
                ::core::result::Result::Ok(())
            }
        }
    })
}

pub fn decode(input: &DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let krate = krate();
    let life = decode_lifetime(&input.generics)?;

    let body = match &input.data {
        Data::Struct(data) => decode_struct(data)?,
        Data::Enum(data) => decode_enum(name, data)?,
        Data::Union(_) => return Err(unsupported(input, "a union has no wire form")),
    };

    let mut generics = input.generics.clone();
    bound_type_params(&mut generics, parse_quote!(#krate::Decode<#life>));
    let (_, ty_generics, where_clause) = generics.split_for_impl();
    let impl_generics = decode_impl_generics(&generics, &life);

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Decode<#life> for #name #ty_generics #where_clause {
            fn decode(reader: &mut #krate::Reader<#life>) -> #krate::Result<Self> {
                ::core::result::Result::Ok(#body)
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

fn encode_struct(data: &DataStruct) -> Result<TokenStream> {
    let mut out = TokenStream::new();
    for (index, field) in data.fields.iter().enumerate() {
        let options = attr::Field::parse(&field.attrs)?;
        let place = field.ident.as_ref().map_or_else(
            || {
                let index = Index::from(index);
                quote!(self.#index)
            },
            |ident| quote!(self.#ident),
        );
        out.extend(encode_value(&quote!(&#place), &field.ty, &options)?);
    }
    Ok(out)
}

fn decode_struct(data: &DataStruct) -> Result<TokenStream> {
    match &data.fields {
        Fields::Named(fields) => {
            let values = fields
                .named
                .iter()
                .map(|field| {
                    let ident = field.ident.as_ref().expect("named field");
                    let value = decode_value(&field.ty, &attr::Field::parse(&field.attrs)?)?;
                    Ok(quote!(#ident: #value))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(quote!(Self { #(#values,)* }))
        }
        Fields::Unnamed(fields) => {
            let values = fields
                .unnamed
                .iter()
                .map(|field| decode_value(&field.ty, &attr::Field::parse(&field.attrs)?))
                .collect::<Result<Vec<_>>>()?;
            Ok(quote!(Self(#(#values,)*)))
        }
        Fields::Unit => Ok(quote!(Self)),
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Discriminants in declaration order, filling in the implicit ones the way
/// Rust does: each unannotated variant is one past the previous.
fn discriminants(data: &DataEnum) -> Result<Vec<(&syn::Ident, i32)>> {
    let mut out = Vec::with_capacity(data.variants.len());
    let mut next = 0i32;
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new(
                variant.span(),
                "a variant with fields needs a hand-written codec: the wire form of the \
                 discriminant does not say how to read what follows",
            ));
        }
        if let Some((_, expr)) = &variant.discriminant {
            let syn::Expr::Lit(lit) = expr else {
                return Err(syn::Error::new(expr.span(), "expected an integer literal"));
            };
            let syn::Lit::Int(int) = &lit.lit else {
                return Err(syn::Error::new(expr.span(), "expected an integer literal"));
            };
            next = int.base10_parse()?;
        }
        out.push((&variant.ident, next));
        next = next.checked_add(1).ok_or_else(|| {
            syn::Error::new(variant.span(), "discriminant overflows the wire's `i32`")
        })?;
    }
    Ok(out)
}

fn encode_enum(data: &DataEnum) -> Result<TokenStream> {
    let arms = discriminants(data)?
        .into_iter()
        .map(|(variant, value)| quote!(Self::#variant => #value));
    // A match rather than `*self as i32`, so the derive works on an enum that
    // is neither `Copy` nor `#[repr(i32)]`.
    Ok(quote! {
        writer.var_int(match self { #(#arms,)* });
    })
}

fn decode_enum(name: &syn::Ident, data: &DataEnum) -> Result<TokenStream> {
    let krate = krate();
    let label = name.to_string();
    let arms = discriminants(data)?
        .into_iter()
        .map(|(variant, value)| quote!(#value => Self::#variant));
    Ok(quote! {
        match reader.var_int()? {
            #(#arms,)*
            value => {
                return ::core::result::Result::Err(#krate::Error::InvalidEnum {
                    name: #label,
                    value,
                });
            }
        }
    })
}

// ---------------------------------------------------------------------------
// One field
// ---------------------------------------------------------------------------

/// A field type as far as the derive needs to understand it.
enum Shape<'a> {
    /// `Option<T>`: a boolean, then the value when present.
    Option(&'a Type),
    /// `Vec<T>`: a `VarInt` count, then that many elements.
    Vec(&'a Type),
    /// `&str`, whose length limit the derive can apply directly.
    Str,
    /// `&[u8]`, whose limit is a byte count rather than a character count.
    Bytes,
    /// Anything else. This is where the walk stops, so it is both the type
    /// that carries its own `Encode`/`Decode` and the integer an encoding
    /// attribute was reaching for.
    Opaque,
}

fn shape(ty: &Type) -> Shape<'_> {
    if let Type::Reference(reference) = ty {
        return match reference.elem.as_ref() {
            Type::Path(path) if path.path.is_ident("str") => Shape::Str,
            Type::Slice(slice) => match slice.elem.as_ref() {
                Type::Path(path) if path.path.is_ident("u8") => Shape::Bytes,
                _ => Shape::Opaque,
            },
            _ => Shape::Opaque,
        };
    }
    let Type::Path(path) = ty else {
        return Shape::Opaque;
    };
    let Some(segment) = path.path.segments.last() else {
        return Shape::Opaque;
    };
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Shape::Opaque;
    };
    let [syn::GenericArgument::Type(inner)] = args.args.iter().collect::<Vec<_>>()[..] else {
        return Shape::Opaque;
    };
    if segment.ident == "Option" {
        Shape::Option(inner)
    } else if segment.ident == "Vec" {
        Shape::Vec(inner)
    } else {
        Shape::Opaque
    }
}

fn encode_value(place: &TokenStream, ty: &Type, options: &attr::Field) -> Result<TokenStream> {
    let krate = krate();
    if let Some(path) = &options.with {
        return Ok(quote!(#path::encode(#place, writer)?;));
    }
    if options.reach.is_empty() {
        return Ok(quote!(#krate::Encode::encode(#place, writer)?;));
    }
    // Bound to a name first: the place is an expression like `&self.pages`,
    // and `&self.pages.iter()` would borrow the iterator rather than the field.
    let body = encode_reaching(&quote!(value), ty, options.reach, 0)?;
    Ok(quote! {
        {
            let value = #place;
            #body
        }
    })
}

/// Encode a field by walking into its type until every option has found the
/// type it describes.
///
/// `place` is always a reference, at every depth: the field is borrowed on the
/// way in, `Option::as_ref` and `slice::iter` keep it so.
fn encode_reaching(
    place: &TokenStream,
    ty: &Type,
    reach: attr::Reach,
    depth: usize,
) -> Result<TokenStream> {
    let krate = krate();
    match shape(ty) {
        Shape::Option(inner) => {
            let binding = format_ident!("value_{}", depth);
            let body = encode_reaching(&quote!(#binding), inner, reach, depth + 1)?;
            Ok(quote! {
                writer.bool(#place.is_some());
                if let ::core::option::Option::Some(#binding) = #place.as_ref() {
                    #body
                }
            })
        }
        Shape::Vec(inner) => {
            let binding = format_ident!("item_{}", depth);
            let body = encode_reaching(&quote!(#binding), inner, reach.count_applied(), depth + 1)?;
            let max = count_limit(reach.max_count);
            Ok(quote! {
                #krate::codec::write_count(writer, #place.len(), #max)?;
                for #binding in #place.iter() {
                    #body
                }
            })
        }
        Shape::Str => {
            let max = require_len(reach, ty, "a string")?;
            Ok(quote!(writer.string_with_limit(#place, #max)?;))
        }
        Shape::Bytes => {
            let max = require_len(reach, ty, "a byte slice")?;
            Ok(quote!(writer.byte_array_with_limit(#place, #max)?;))
        }
        Shape::Opaque => match reach.encoding {
            Some(encoding) => {
                unapplied(reach.encoding_applied(), ty, "an integer")?;
                let write = format_ident!("{}", encoding_method(encoding));
                Ok(quote!(writer.#write(*#place);))
            }
            None => {
                unapplied(reach, ty, "a type with a codec of its own")?;
                Ok(quote!(#krate::Encode::encode(#place, writer)?;))
            }
        },
    }
}

fn decode_value(ty: &Type, options: &attr::Field) -> Result<TokenStream> {
    let krate = krate();
    if let Some(path) = &options.with {
        return Ok(quote!(#path::decode(reader)?));
    }
    if options.reach.is_empty() {
        return Ok(quote!(#krate::Decode::decode(reader)?));
    }
    decode_reaching(ty, options.reach)
}

/// Decode a field by the same walk [`encode_reaching`] makes, so that the two
/// stop at the same type and read what the other wrote.
fn decode_reaching(ty: &Type, reach: attr::Reach) -> Result<TokenStream> {
    let krate = krate();
    match shape(ty) {
        Shape::Option(inner) => {
            let body = decode_reaching(inner, reach)?;
            Ok(quote! {
                if reader.bool()? {
                    ::core::option::Option::Some(#body)
                } else {
                    ::core::option::Option::None
                }
            })
        }
        Shape::Vec(inner) => {
            let body = decode_reaching(inner, reach.count_applied())?;
            let max = count_limit(reach.max_count);
            // The capacity is clamped to the bytes actually available, because
            // the count is attacker-controlled and every element costs at
            // least one byte.
            Ok(quote! {
                {
                    let count = #krate::codec::read_count(reader, #max)?;
                    let mut items = ::std::vec::Vec::with_capacity(
                        ::core::cmp::min(count, reader.remaining_len()),
                    );
                    for _ in 0..count {
                        items.push(#body);
                    }
                    items
                }
            })
        }
        Shape::Str => {
            let max = require_len(reach, ty, "a string")?;
            Ok(quote!(reader.string_with_limit(#max)?))
        }
        Shape::Bytes => {
            let max = require_len(reach, ty, "a byte slice")?;
            Ok(quote!(reader.byte_array_with_limit(#max)?))
        }
        Shape::Opaque => match reach.encoding {
            Some(encoding) => {
                unapplied(reach.encoding_applied(), ty, "an integer")?;
                let read = format_ident!("{}", encoding_method(encoding));
                Ok(quote!(reader.#read()?))
            }
            None => {
                unapplied(reach, ty, "a type with a codec of its own")?;
                Ok(quote!(#krate::Decode::decode(reader)?))
            }
        },
    }
}

/// The `Reader` and `Writer` method for an encoding. They share a name, which
/// is what lets one lookup serve both directions.
const fn encoding_method(encoding: attr::Encoding) -> &'static str {
    match encoding {
        attr::Encoding::VarInt => "var_int",
        attr::Encoding::VarLong => "var_long",
    }
}

fn require_len(reach: attr::Reach, ty: &Type, leaf: &str) -> Result<usize> {
    // Whatever else was asked for is reported first: it names a type this
    // field does not contain, which is the more surprising of the two.
    unapplied(reach.len_applied(), ty, leaf)?;
    reach.max_len.ok_or_else(|| {
        syn::Error::new(
            ty.span(),
            format!("this field bottoms out in {leaf}, whose limit is `max_len`"),
        )
    })
}

/// Refuse an option that reached the bottom of a field without finding the
/// type it describes.
///
/// Dropping one silently would produce a codec that looks constrained and is
/// not, which is the failure this derive exists to rule out.
fn unapplied(reach: attr::Reach, ty: &Type, leaf: &str) -> Result<()> {
    let name = if reach.max_len.is_some() {
        "`max_len`"
    } else if reach.max_count.is_some() {
        "`max_count`"
    } else if reach.encoding.is_some() {
        "`varint`/`varlong`"
    } else {
        return Ok(());
    };
    Err(syn::Error::new(
        ty.span(),
        format!("{name} found nothing to apply to: this field bottoms out in {leaf}"),
    ))
}

fn count_limit(max: Option<usize>) -> TokenStream {
    max.map_or_else(
        || quote!(::core::option::Option::None),
        |max| quote!(::core::option::Option::Some(#max)),
    )
}

// ---------------------------------------------------------------------------
// Generics
// ---------------------------------------------------------------------------

/// The lifetime the decoded value borrows from.
///
/// One lifetime is the borrowed case and none is the owned case. Two would
/// mean the derive had to decide which of them the reader lends to, and
/// guessing wrong is a borrow error a long way from its cause.
fn decode_lifetime(generics: &Generics) -> Result<Lifetime> {
    let mut lifetimes = generics.lifetimes();
    let first = lifetimes.next();
    if let Some(extra) = lifetimes.next() {
        return Err(syn::Error::new(
            extra.span(),
            "`Decode` needs one lifetime to borrow from, and this type has several",
        ));
    }
    Ok(first.map_or_else(
        || Lifetime::new("'de", Span::call_site()),
        |param| param.lifetime.clone(),
    ))
}

fn decode_impl_generics(generics: &Generics, life: &Lifetime) -> Generics {
    let mut out = generics.clone();
    if !generics.lifetimes().any(|param| param.lifetime == *life) {
        out.params
            .insert(0, GenericParam::Lifetime(LifetimeParam::new(life.clone())));
    }
    out
}

fn bound_type_params(generics: &mut Generics, bound: TypeParamBound) {
    for param in &mut generics.params {
        if let GenericParam::Type(param) = param {
            param.bounds.push(bound.clone());
        }
    }
}

fn unsupported(input: &DeriveInput, why: &str) -> syn::Error {
    syn::Error::new(input.span(), why)
}

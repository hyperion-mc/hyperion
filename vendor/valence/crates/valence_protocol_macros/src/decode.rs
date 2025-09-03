use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    parse2, parse_quote, Data, DeriveInput, Error, Fields, PredicateType, Result, Token,
    TraitBound, TraitBoundModifier, TypeParamBound, WhereClause, WherePredicate,
};

use crate::{add_trait_bounds, pair_variants_with_discriminants};

pub(super) fn derive_decode(item: TokenStream) -> Result<TokenStream> {
    let mut input = parse2::<DeriveInput>(item)?;

    let input_name = input.ident;

    match input.data {
        Data::Struct(struct_) => {
            let decode_fields = match struct_.fields {
                Fields::Named(fields) => {
                    let init = fields.named.iter().map(|f| {
                        let name = f.ident.as_ref().unwrap();
                        let ctx = format!("failed to decode field `{name}` in `{input_name}`");
                        quote! {
                            #name: Decode::decode(_r).context(#ctx)?,
                        }
                    });

                    quote! {
                        Self {
                            #(#init)*
                        }
                    }
                }
                Fields::Unnamed(fields) => {
                    let init = (0..fields.unnamed.len())
                        .map(|i| {
                            let ctx = format!("failed to decode field `{i}` in `{input_name}`");
                            quote! {
                                Decode::decode(_r).context(#ctx)?,
                            }
                        })
                        .collect::<TokenStream>();

                    quote! {
                        Self(#init)
                    }
                }
                Fields::Unit => quote!(Self),
            };

            add_trait_bounds(
                &mut input.generics,
                quote!(::valence_protocol::__private::Decode),
            );

            let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

            Ok(quote! {
                #[allow(unused_imports)]
                impl #impl_generics ::valence_protocol::__private::Decode for #input_name #ty_generics
                #where_clause
                {
                    fn decode(_r: &mut &[u8]) -> ::valence_protocol::__private::Result<Self> {
                        use ::valence_protocol::__private::{Decode, Context, ensure};

                        Ok(#decode_fields)
                    }
                }
            })
        }
        Data::Enum(enum_) => {
            let variants = pair_variants_with_discriminants(enum_.variants)?;

            let decode_arms = variants
                .iter()
                .map(|(disc, variant)| {
                    let name = &variant.ident;

                    match &variant.fields {
                        Fields::Named(fields) => {
                            let fields = fields
                                .named
                                .iter()
                                .map(|f| {
                                    let field = f.ident.as_ref().unwrap();
                                    let ctx = format!(
                                        "failed to decode field `{field}` in variant `{name}` in \
                                         `{input_name}`",
                                    );
                                    quote! {
                                        #field: Decode::decode(_r).context(#ctx)?,
                                    }
                                })
                                .collect::<TokenStream>();

                            quote! {
                                #disc => Ok(Self::#name { #fields }),
                            }
                        }
                        Fields::Unnamed(fields) => {
                            let init = (0..fields.unnamed.len())
                                .map(|i| {
                                    let ctx = format!(
                                        "failed to decode field `{i}` in variant `{name}` in \
                                         `{input_name}`",
                                    );
                                    quote! {
                                        Decode::decode(_r).context(#ctx)?,
                                    }
                                })
                                .collect::<TokenStream>();

                            quote! {
                                #disc => Ok(Self::#name(#init)),
                            }
                        }
                        Fields::Unit => quote!(#disc => Ok(Self::#name),),
                    }
                })
                .collect::<TokenStream>();

            add_trait_bounds(
                &mut input.generics,
                quote!(::valence_protocol::__private::Decode),
            );

            let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

            Ok(quote! {
                #[allow(unused_imports)]
                impl #impl_generics ::valence_protocol::__private::Decode for #input_name #ty_generics
                #where_clause
                {
                    fn decode(_r: &mut &[u8]) -> ::valence_protocol::__private::Result<Self> {
                        use ::valence_protocol::__private::{Decode, Context, VarInt, bail};

                        let ctx = concat!("failed to decode enum discriminant in `", stringify!(#input_name), "`");
                        let disc = VarInt::decode(_r).context(ctx)?.0;
                        match disc {
                            #decode_arms
                            n => bail!("unexpected enum discriminant {} in `{}`", disc, stringify!(#input_name)),
                        }
                    }
                }
            })
        }
        Data::Union(u) => Err(Error::new(
            u.union_token.span(),
            "cannot derive `Decode` on unions",
        )),
    }
}

pub(super) fn derive_decode_bytes(item: TokenStream) -> Result<TokenStream> {
    let mut input = parse2::<DeriveInput>(item)?;

    let input_name = input.ident;

    match input.data {
        Data::Struct(struct_) => {
            let decode_fields = match struct_.fields {
                Fields::Named(fields) => {
                    let init = fields.named.iter().map(|f| {
                        let name = f.ident.as_ref().unwrap();
                        let ctx = format!("failed to decode field `{name}` in `{input_name}`");
                        quote! {
                            #name: DecodeBytes::decode_bytes(_r).context(#ctx)?,
                        }
                    });

                    quote! {
                        Self {
                            #(#init)*
                        }
                    }
                }
                Fields::Unnamed(fields) => {
                    let init = (0..fields.unnamed.len())
                        .map(|i| {
                            let ctx = format!("failed to decode field `{i}` in `{input_name}`");
                            quote! {
                                DecodeBytes::decode_bytes(_r).context(#ctx)?,
                            }
                        })
                        .collect::<TokenStream>();

                    quote! {
                        Self(#init)
                    }
                }
                Fields::Unit => quote!(Self),
            };

            add_trait_bounds(
                &mut input.generics,
                quote!(::valence_protocol::__private::DecodeBytes),
            );

            let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

            Ok(quote! {
                #[allow(unused_imports)]
                impl #impl_generics ::valence_protocol::__private::DecodeBytes for #input_name #ty_generics
                #where_clause
                {
                    fn decode_bytes(_r: &mut ::valence_protocol::__private::Bytes) -> ::valence_protocol::__private::Result<Self> {
                        use ::valence_protocol::__private::{DecodeBytes, Context, ensure};

                        Ok(#decode_fields)
                    }
                }
            })
        }
        Data::Enum(enum_) => {
            let variants = pair_variants_with_discriminants(enum_.variants)?;

            let decode_arms = variants
                .iter()
                .map(|(disc, variant)| {
                    let name = &variant.ident;

                    match &variant.fields {
                        Fields::Named(fields) => {
                            let fields = fields
                                .named
                                .iter()
                                .map(|f| {
                                    let field = f.ident.as_ref().unwrap();
                                    let ctx = format!(
                                        "failed to decode field `{field}` in variant `{name}` in \
                                         `{input_name}`",
                                    );
                                    quote! {
                                        #field: DecodeBytes::decode_bytes(_r).context(#ctx)?,
                                    }
                                })
                                .collect::<TokenStream>();

                            quote! {
                                #disc => Ok(Self::#name { #fields }),
                            }
                        }
                        Fields::Unnamed(fields) => {
                            let init = (0..fields.unnamed.len())
                                .map(|i| {
                                    let ctx = format!(
                                        "failed to decode field `{i}` in variant `{name}` in \
                                         `{input_name}`",
                                    );
                                    quote! {
                                        DecodeBytes::decode_bytes(_r).context(#ctx)?,
                                    }
                                })
                                .collect::<TokenStream>();

                            quote! {
                                #disc => Ok(Self::#name(#init)),
                            }
                        }
                        Fields::Unit => quote!(#disc => Ok(Self::#name),),
                    }
                })
                .collect::<TokenStream>();

            add_trait_bounds(
                &mut input.generics,
                quote!(::valence_protocol::__private::DecodeBytes),
            );

            let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

            Ok(quote! {
                #[allow(unused_imports)]
                impl #impl_generics ::valence_protocol::__private::DecodeBytes for #input_name #ty_generics
                #where_clause
                {
                    fn decode_bytes(_r: &mut ::valence_protocol::__private::Bytes) -> ::valence_protocol::__private::Result<Self> {
                        use ::valence_protocol::__private::{DecodeBytes, Context, VarInt, bail};

                        let ctx = concat!("failed to decode enum discriminant in `", stringify!(#input_name), "`");
                        let disc = VarInt::decode_bytes(_r).context(ctx)?.0;
                        match disc {
                            #decode_arms
                            n => bail!("unexpected enum discriminant {} in `{}`", disc, stringify!(#input_name)),
                        }
                    }
                }
            })
        }
        Data::Union(u) => Err(Error::new(
            u.union_token.span(),
            "cannot derive `DecodeBytes` on unions",
        )),
    }
}

pub(super) fn derive_decode_bytes_auto(item: TokenStream) -> Result<TokenStream> {
    let input = parse2::<DeriveInput>(item)?;

    let input_name = input.ident;

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let mut where_clause = where_clause.cloned().unwrap_or_else(|| WhereClause {
        where_token: <Token![where]>::default(),
        predicates: Punctuated::new(),
    });

    // Require the type to implement Decode
    where_clause
        .predicates
        .push(WherePredicate::Type(PredicateType {
            lifetimes: None,
            bounded_ty: parse_quote!(#input_name #ty_generics),
            colon_token: <Token![:]>::default(),
            bounds: Punctuated::from_iter(std::iter::once(TypeParamBound::Trait(TraitBound {
                paren_token: None,
                modifier: TraitBoundModifier::None,
                lifetimes: None,
                path: parse_quote!(::valence_protocol::__private::Decode),
            }))),
        }));

    Ok(quote! {
        impl #impl_generics ::valence_protocol::__private::DecodeBytes for #input_name #ty_generics
        #where_clause
        {
            fn decode_bytes(r: &mut ::valence_protocol::__private::Bytes) -> ::valence_protocol::__private::Result<Self> {
                ::valence_protocol::__private::decode_bytes_auto(r)
            }

            fn decode_from_owned<T>(r: T) -> ::anyhow::Result<(Self, usize)> where T: ::std::convert::AsRef<[u8]> + ::std::marker::Send + 'static {
                <Self as ::valence_protocol::__private::Decode>::decode_and_len(r.as_ref())
            }
        }
    })
}

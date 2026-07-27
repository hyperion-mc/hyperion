//! Parsing of the `#[proto(...)]` field attribute.

use syn::{Attribute, Expr, LitInt, Path, Result, spanned::Spanned};

/// What a `#[proto(...)]` attribute says about one field.
#[derive(Default)]
pub struct Field {
    /// `max_len = N`: the innermost string's limit in UTF-16 code units.
    pub max_len: Option<usize>,
    /// `max_count = N`: the innermost collection's element limit.
    pub max_count: Option<usize>,
    /// `with = path`: a module supplying `encode` and `decode` for this field.
    pub with: Option<Path>,
}

impl Field {
    /// Read the single `#[proto(...)]` attribute a field may carry.
    pub fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("proto")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("max_len") {
                    out.max_len = Some(usize_value(&meta.value()?.parse()?)?);
                } else if meta.path.is_ident("max_count") {
                    out.max_count = Some(usize_value(&meta.value()?.parse()?)?);
                } else if meta.path.is_ident("with") {
                    out.with = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error("expected `max_len`, `max_count` or `with`"));
                }
                Ok(())
            })?;
        }
        if out.with.is_some() && (out.max_len.is_some() || out.max_count.is_some()) {
            // A `with` module owns the whole field, so a limit beside it would
            // be silently ignored rather than enforced somewhere unexpected.
            return Err(syn::Error::new(
                attrs[0].span(),
                "`with` supplies the whole codec, so it cannot be combined with a limit",
            ));
        }
        Ok(out)
    }
}

fn usize_value(expr: &Expr) -> Result<usize> {
    let Expr::Lit(lit) = expr else {
        return Err(syn::Error::new(expr.span(), "expected an integer literal"));
    };
    let syn::Lit::Int(int) = &lit.lit else {
        return Err(syn::Error::new(expr.span(), "expected an integer literal"));
    };
    LitInt::base10_parse::<usize>(int)
}

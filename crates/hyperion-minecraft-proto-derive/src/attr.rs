//! Parsing of the `#[proto(...)]` field attribute.

use syn::{Attribute, Expr, LitInt, Path, Result, spanned::Spanned};

/// How an integer is written, where the wire's choice is not the one its Rust
/// type implies.
///
/// How wide a number is belongs to the value and so lives in the type; how
/// many bytes it costs on the wire belongs to the wire and lives here.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// An `i32`, seven bits per byte.
    VarInt,
    /// An `i64`, seven bits per byte.
    VarLong,
}

/// What a `#[proto(...)]` attribute asks of the types *inside* a field.
///
/// Each of these applies to one type nested somewhere under the field's own,
/// so the derive walks the field type until it finds the type each describes.
#[derive(Clone, Copy, Default)]
pub struct Reach {
    /// `max_len = N`: the innermost string's limit in UTF-16 code units.
    pub max_len: Option<usize>,
    /// `max_count = N`: the innermost collection's element limit.
    pub max_count: Option<usize>,
    /// `varint` or `varlong`: how the innermost integer is written.
    pub encoding: Option<Encoding>,
}

impl Reach {
    /// Whether anything here still has to reach a type inside the field's own.
    pub const fn is_empty(self) -> bool {
        self.max_len.is_none() && self.max_count.is_none() && self.encoding.is_none()
    }

    /// The same options with the collection limit spent, for the element type
    /// of the list that just applied it.
    pub const fn count_applied(self) -> Self {
        Self {
            max_count: None,
            ..self
        }
    }

    /// What is left once a string or byte slice has taken its length limit.
    pub const fn len_applied(self) -> Self {
        Self {
            max_len: None,
            ..self
        }
    }

    /// What is left once an integer has taken its encoding.
    pub const fn encoding_applied(self) -> Self {
        Self {
            encoding: None,
            ..self
        }
    }
}

/// What a `#[proto(...)]` attribute says about one field.
#[derive(Default)]
pub struct Field {
    /// What has to reach a type nested inside this field's own.
    pub reach: Reach,
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
                    out.reach.max_len = Some(usize_value(&meta.value()?.parse()?)?);
                } else if meta.path.is_ident("max_count") {
                    out.reach.max_count = Some(usize_value(&meta.value()?.parse()?)?);
                } else if meta.path.is_ident("varint") {
                    out.set_encoding(Encoding::VarInt, &meta)?;
                } else if meta.path.is_ident("varlong") {
                    out.set_encoding(Encoding::VarLong, &meta)?;
                } else if meta.path.is_ident("with") {
                    out.with = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta
                        .error("expected `varint`, `varlong`, `max_len`, `max_count` or `with`"));
                }
                Ok(())
            })?;
        }
        if out.with.is_some() && !out.reach.is_empty() {
            // A `with` module owns the whole field, so anything beside it
            // would be silently ignored rather than applied somewhere
            // unexpected.
            return Err(syn::Error::new(
                attrs[0].span(),
                "`with` supplies the whole codec, so nothing else can apply to the field",
            ));
        }
        if out.reach.max_len.is_some() && out.reach.encoding.is_some() {
            // Both describe the one type at the bottom of the field, and no
            // type is both a string and an integer.
            return Err(syn::Error::new(
                attrs[0].span(),
                "`max_len` describes a string and `varint`/`varlong` an integer, so one field \
                 cannot want both",
            ));
        }
        Ok(out)
    }

    fn set_encoding(
        &mut self,
        encoding: Encoding,
        meta: &syn::meta::ParseNestedMeta<'_>,
    ) -> Result<()> {
        if self.reach.encoding.is_some_and(|already| already != encoding) {
            return Err(meta.error("a value has one wire encoding, and two are named here"));
        }
        self.reach.encoding = Some(encoding);
        Ok(())
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

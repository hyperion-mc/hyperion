//! `#[derive(CommandPermission)]`: which group a command, and each of its
//! subcommands, is for.
//!
//! # A group per subcommand, not per command
//!
//! This derive used to emit one group for the whole type. On a `clap` enum,
//! where each variant is a subcommand, that made the weakest variant the gate
//! for all of them, and `/perms` was declared `Normal` because `/perms get` is:
//! every player who could connect could run `/perms set <themselves> Admin`
//! (ENG-10871).
//!
//! So a group is declared on the type, on a variant, or on both:
//!
//! ```ignore
//! #[derive(Parser, CommandPermission)]
//! #[command(name = "perms")]
//! pub enum PermissionCommand {
//!     #[command_permission(group = "Admin")]
//!     Set(SetCommand),
//!     #[command_permission(group = "Normal")]
//!     Get(GetCommand),
//! }
//! ```
//!
//! A type-level group is the default for variants that do not declare their
//! own, which is how a command whose subcommands are all equally dangerous
//! stays one line.
//!
//! # Declaring some but not all of them is a compile error
//!
//! What is refused is the shape that produced the bug: no type-level group and
//! only some variants carrying one. There the undeclared variants would have to
//! fall back to something, and every available fallback is a guess about
//! whether the author meant "same as the others" or forgot. The error names the
//! variants that are missing.
//!
//! # Where the generated code is used
//!
//! `hyperion_clap::MinecraftCommand::register` asks three separate questions,
//! and getting the wrong one is how this bug was reachable in the first place:
//!
//! * `has_required_permission` -- may this player use the command at all,
//!   which is true when they may use *any* of its subcommands. It decides
//!   whether the command's own node is in the tree, and `CommandRegistry`
//!   lists it.
//! * `subcommand_has_required_permission` -- may this player see this
//!   subcommand's literal in their command tree.
//! * `variant_has_required_permission` -- may this player run *this* parsed
//!   invocation. This is the one that gates execution, and the only one that
//!   is a security boundary.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, Ident, Lit, LitStr, Variant, parse_macro_input,
    spanned::Spanned,
};

/// The group named by a `#[command_permission(group = "...")]` on `attrs`.
fn declared_group(attrs: &[Attribute]) -> syn::Result<Option<Ident>> {
    let mut group = None;

    for attr in attrs {
        if !attr.path().is_ident("command_permission") {
            continue;
        }

        let mut found: Option<LitStr> = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("group") {
                if let Lit::Str(lit) = meta.value()?.parse::<Lit>()? {
                    found = Some(lit);
                    return Ok(());
                }
                return Err(meta.error("group takes a string, such as group = \"Admin\""));
            }
            Err(meta.error("the only key is `group`"))
        })?;

        let Some(found) = found else {
            return Err(Error::new_spanned(
                attr,
                "`#[command_permission]` needs a group: `#[command_permission(group = \"Admin\")]`",
            ));
        };

        group = Some(Ident::new(&found.value(), found.span()));
    }

    Ok(group)
}

/// Refuse a `#[command_permission]` written somewhere it would be ignored.
///
/// A derive helper attribute on a field parses and then does nothing, so
/// without this a group declared one level too deep is a command that silently
/// keeps the group above it.
fn reject_group_on_fields(fields: &Fields) -> syn::Result<()> {
    for field in fields {
        if field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("command_permission"))
        {
            return Err(Error::new(
                field.span(),
                "a group belongs on the command or on one of its subcommands, not on an argument; \
                 a `#[command_permission]` here would be ignored",
            ));
        }
    }
    Ok(())
}

/// What `clap` calls this variant on the command line.
///
/// `clap_derive` kebab-cases the variant's name unless `#[command(name = ...)]`
/// says otherwise, and this has to arrive at the same answer or the group would
/// be filed under a subcommand that does not exist. It is not trusted to:
/// `MinecraftCommand::register` checks every name `clap` reports against
/// `declared_subcommands` and refuses to start the server if one is missing,
/// so a disagreement is a startup failure naming both spellings rather than a
/// subcommand quietly falling back to the command's own group.
fn clap_name(variant: &Variant) -> String {
    for attr in &variant.attrs {
        if !attr.path().is_ident("command") && !attr.path().is_ident("clap") {
            continue;
        }

        let mut named = None;
        // Every other `command` key is clap's business, so an unrecognised one
        // is skipped rather than rejected.
        drop(attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name")
                && let Ok(value) = meta.value()
                && let Ok(Lit::Str(lit)) = value.parse::<Lit>()
            {
                named = Some(lit.value());
            }
            Ok(())
        }));

        if let Some(named) = named {
            return named;
        }
    }

    kebab_case(&variant.ident.to_string())
}

/// `heck`'s kebab-casing, for the identifiers a clap subcommand can be.
fn kebab_case(ident: &str) -> String {
    let chars: Vec<char> = ident.chars().collect();
    let mut out = String::with_capacity(chars.len() + 2);

    for (index, &current) in chars.iter().enumerate() {
        if current == '_' {
            out.push('-');
            continue;
        }

        if current.is_uppercase() && index != 0 {
            let previous = chars[index - 1];
            let next_is_lower = chars.get(index + 1).is_some_and(|c| c.is_lowercase());
            if previous.is_lowercase()
                || previous.is_numeric()
                || (previous.is_uppercase() && next_is_lower)
            {
                out.push('-');
            }
        }

        out.extend(current.to_lowercase());
    }

    out
}

/// `user_group.allows(Group::#group)`, which is the whole permission rule.
fn allows(group: &Ident) -> proc_macro2::TokenStream {
    quote! {
        ::hyperion_permission::Group::allows(user_group, ::hyperion_permission::Group::#group)
    }
}

#[proc_macro_derive(CommandPermission, attributes(command_permission))]
pub fn derive_command_permission(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let default_group = declared_group(&input.attrs)?;

    let (any, variant, subcommand, declared) = match &input.data {
        Data::Enum(data) => {
            for variant in &data.variants {
                reject_group_on_fields(&variant.fields)?;
            }
            enum_arms(name, default_group.as_ref(), data)?
        }
        Data::Struct(data) => {
            reject_group_on_fields(&data.fields)?;
            let Some(group) = default_group else {
                return Err(Error::new_spanned(
                    input,
                    "missing `#[command_permission(group = \"<GroupName>\")]`, which says who \
                     this command is for",
                ));
            };
            let check = allows(&group);
            (
                check,
                quote! { <Self as CommandPermission>::has_required_permission(user_group) },
                quote! { <Self as CommandPermission>::has_required_permission(user_group) },
                Vec::new(),
            )
        }
        Data::Union(_) => {
            return Err(Error::new_spanned(
                input,
                "a command is a struct or an enum, not a union",
            ));
        }
    };

    Ok(quote! {
        impl CommandPermission for #name {
            fn has_required_permission(user_group: ::hyperion_permission::Group) -> bool {
                #any
            }

            fn variant_has_required_permission(
                &self,
                user_group: ::hyperion_permission::Group,
            ) -> bool {
                #variant
            }

            fn subcommand_has_required_permission(
                subcommand: ::core::option::Option<&str>,
                user_group: ::hyperion_permission::Group,
            ) -> bool {
                #subcommand
            }

            fn declared_subcommands() -> &'static [&'static str] {
                &[#(#declared),*]
            }
        }
    })
}

type Arms = (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    Vec<String>,
);

fn enum_arms(
    name: &Ident,
    default_group: Option<&Ident>,
    data: &syn::DataEnum,
) -> syn::Result<Arms> {
    if data.variants.is_empty() {
        return Err(Error::new_spanned(
            name,
            "a command with no subcommands has nothing to gate",
        ));
    }

    let mut groups = Vec::new();
    let mut undeclared = Vec::new();

    for variant in &data.variants {
        match declared_group(&variant.attrs)?.or_else(|| default_group.cloned()) {
            Some(group) => groups.push((variant, group)),
            None => undeclared.push(variant.ident.to_string()),
        }
    }

    if !undeclared.is_empty() {
        // The shape that produced ENG-10871, refused rather than guessed at.
        return Err(Error::new_spanned(
            name,
            format!(
                "these subcommands declare no group and the command declares no default for them: \
                 {undeclared:?}. Either give the command a `#[command_permission(group = ...)]` \
                 covering every subcommand, or give each subcommand its own. Declaring only some \
                 is refused because the fallback would be a guess, and the guess that used to be \
                 made here let any player run `/perms set` (ENG-10871)."
            ),
        ));
    }

    let any_checks: Vec<_> = groups.iter().map(|(_, group)| allows(group)).collect();

    let variant_arms: Vec<_> = groups
        .iter()
        .map(|(variant, group)| {
            let ident = &variant.ident;
            let check = allows(group);
            quote! { Self::#ident { .. } => #check }
        })
        .collect();

    let mut declared = Vec::new();
    let mut subcommand_arms = Vec::new();
    for (variant, group) in &groups {
        let literal = clap_name(variant);
        let check = allows(group);
        subcommand_arms.push(quote! { ::core::option::Option::Some(#literal) => #check });
        declared.push(literal);
    }

    Ok((
        // May this player use the command at all: true when any one of its
        // subcommands is open to them.
        quote! { #(#any_checks)||* },
        quote! { match self { #(#variant_arms),* } },
        quote! {
            match subcommand {
                // The command's own node, which is in the tree when any
                // subcommand under it is.
                ::core::option::Option::None => {
                    <Self as CommandPermission>::has_required_permission(user_group)
                }
                #(#subcommand_arms,)*
                // Unreachable: `register` refuses to start a server whose clap
                // subcommands are not all declared here. Fails closed anyway,
                // because the alternative is the weakest-variant-wins default
                // this derive exists to remove.
                ::core::option::Option::Some(_) => false,
            }
        },
        declared,
    ))
}

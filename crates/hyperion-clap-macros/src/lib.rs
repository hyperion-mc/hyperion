//! Macros to build commands that have permissions. These are on top of the existing
//! `clap::Parser` traits.
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error, Ident, Lit, parse_macro_input};

/// Derives the `CommandPermission` trait for a struct or enum.
#[proc_macro_derive(CommandPermission, attributes(command_permission))]
pub fn derive_command_permission(input: TokenStream) -> TokenStream {
    // Parse the input as a DeriveInput (struct or enum)
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone(); // Clone the Ident to prevent moving

    // Extract the group from the `#[command_permission(group = "Admin")]` attribute
    let mut group = None;
    for attr in &input.attrs {
        if attr.path().is_ident("command_permission") {
            if let Err(err) = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("group") {
                    if let Ok(Lit::Str(lit)) = meta.value()?.parse::<Lit>() {
                        group = Some(lit);
                    }
                }
                Ok(())
            }) {
                return Error::new_spanned(attr, format!("Failed to parse attribute: {err}"))
                    .to_compile_error()
                    .into();
            }
        }
    }

    let group_ident = match group {
        Some(g) => Ident::new(&g.value(), g.span()),
        None => {
            return Error::new_spanned(
                input,
                "Missing required `#[command_permission(group = \"<GroupName>\")]` attribute.",
            )
            .to_compile_error()
            .into();
        }
    };

    // Generate the trait implementation
    let expanded = quote! {
        impl CommandPermission for #name {
            fn has_required_permission(user_group: ::hyperion_permission::Group) -> bool {
                const REQUIRED_GROUP: ::hyperion_permission::Group = ::hyperion_permission::Group::#group_ident;

                if REQUIRED_GROUP == ::hyperion_permission::Group::Banned {
                    // When checking for the group "Banned" we don't want to check for higher groups.
                    return REQUIRED_GROUP == user_group;
                } else {
                    return user_group as u32 >= REQUIRED_GROUP as u32
                }
            }
        }
    };

    TokenStream::from(expanded)
}

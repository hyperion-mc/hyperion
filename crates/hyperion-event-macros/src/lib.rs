use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Lifetime, Token, Type,
    parse::{Parse, ParseStream, Result},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
};

/// Entry point for the procedural macro
#[proc_macro]
pub fn define_events(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let events = parse_macro_input!(input as EventsInput);

    // Generate the Events struct and implementations
    let expanded = events.generate();

    // Return the generated code as a TokenStream
    TokenStream::from(expanded)
}

/// Struct representing the entire input to the macro
struct EventsInput {
    events: Vec<EventType>,
}

impl Parse for EventsInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let punctuated: Punctuated<Type, Token![,]> = Punctuated::parse_terminated(input)?;
        let mut events = Vec::new();

        for ty in punctuated {
            events.push(EventType::from_type(ty)?);
        }

        Ok(Self { events })
    }
}

/// Struct representing a single event type, possibly with a lifetime
struct EventType {
    path: syn::Ident,
    ty: Type,
    lifetime: Option<Lifetime>,
    ident: syn::Ident,
}

impl EventType {
    /// Extracts necessary components from a `syn::Type`
    fn from_type(ty: Type) -> Result<Self> {
        let mut lifetime = None;

        let Type::Path(type_path) = &ty else {
            return Err(syn::Error::new(ty.span(), "expected a path"));
        };

        let path = type_path.path.clone();

        let Some(last_segment) = path.segments.last() else {
            return Err(syn::Error::new(
                ty.span(),
                "expected a path with at least one segment",
            ));
        };

        let ident = last_segment.ident.clone();
        let generics = last_segment.arguments.clone();

        // Extract lifetime if present
        if let syn::PathArguments::AngleBracketed(args) = &generics {
            for generic_arg in &args.args {
                if let syn::GenericArgument::Lifetime(l) = generic_arg {
                    lifetime = Some(l.clone());
                    break;
                }
            }
        }

        let Some(first_segment) = path.segments.first() else {
            return Err(syn::Error::new(
                path.span(),
                "expected a path with at least one segment",
            ));
        };

        let path = first_segment.ident.clone();

        Ok(Self {
            path,
            ty,
            lifetime,
            ident,
        })
    }

    /// Generates the field definition for the Events struct
    fn generate_field(&self) -> proc_macro2::TokenStream {
        let field_name = self.ident.to_string().to_case(Case::Snake);
        let field_ident = format_ident!("{field_name}");
        let ty = &self.ty;

        quote! {
            pub #field_ident: SendSyncPtr<EventQueue<#ty>>,
        }
    }

    /// Generates the initializer for the Events struct
    fn generate_initializer(&self) -> proc_macro2::TokenStream {
        let field_name = self.ident.to_string().to_case(Case::Snake);
        let field_ident = format_ident!("{field_name}");
        let ty = &self.ty;

        quote! {
            #field_ident: SendSyncPtr(register_and_pointer(world, EventQueue::<#ty>::default()), PhantomData),
        }
    }

    /// Generates the necessary trait implementations for the event type
    fn generate_impls(&self) -> proc_macro2::TokenStream {
        let path = &self.path;
        let ident = &self.ident;
        let field_name = self.ident.to_string().to_case(Case::Snake);
        let field_ident = format_ident!("{field_name}");

        if self.lifetime.is_some() {
            quote! {
                impl Event for #path::#ident<'static> {
                    fn input(elem: Self, events: &Events, world: &World) {
                        unsafe {
                            (*events.#field_ident.0).push(elem, world);
                        }
                    }
                }

                impl sealed::Sealed for #path::#ident<'static> {}

                unsafe impl crate::common::util::Lifetime for #path::#ident<'_> {
                    type WithLifetime<'a> = #path::#ident<'a>;
                }

                impl ReducedLifetime for #path::#ident<'static> {
                    type Reduced<'a> = #path::#ident<'a>
                    where
                        Self: 'a;

                    fn reduce<'a>(self) -> Self::Reduced<'a> {
                        self
                    }
                }
            }
        } else {
            quote! {
                impl Event for #path::#ident {
                    fn input(elem: Self, events: &Events, world: &World) {
                        unsafe {
                            (*events.#field_ident.0).push(elem, world);
                        }
                    }
                }

                impl sealed::Sealed for #path::#ident {}

                unsafe impl crate::common::util::Lifetime for #path::#ident {
                    type WithLifetime<'a> = #path::#ident;
                }

                impl ReducedLifetime for #path::#ident {
                    type Reduced<'a> = Self
                    where
                        Self: 'a;

                    fn reduce<'a>(self) -> Self::Reduced<'a> {
                        self
                    }
                }
            }
        }
    }
}

impl EventsInput {
    /// Generates the complete expanded code for the macro
    fn generate(&self) -> proc_macro2::TokenStream {
        // Generate all fields and initializers
        let fields = self.events.iter().map(EventType::generate_field);

        let field_idents = self.events.iter().map(|event| {
            let field_name = event.ident.to_string().to_case(Case::Snake);
            format_ident!("{field_name}")
        });

        let initializers = self.events.iter().map(EventType::generate_initializer);

        // Generate all trait implementations
        let impls = self.events.iter().map(EventType::generate_impls);

        // Generate the Events struct
        let events_struct = quote! {
            #[derive(Component)]
            pub struct Events {
                #(#fields)*
            }

            impl Events {
                #[must_use]
                pub fn initialize(world: &World) -> Self {
                    Self {
                        #(#initializers)*
                    }
                }

                pub fn clear(&mut self) {
                    #(
                        let ptr = self.#field_idents.0;
                        let ptr = ptr.cast_mut();
                        let ptr = unsafe { &mut *ptr };
                        ptr.clear();
                    )*
                }
            }
        };

        // Combine everything
        quote! {
            #events_struct
            #(#impls)*
        }
    }
}

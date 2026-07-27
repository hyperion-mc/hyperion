//! Hot reloading for flecs game modules, gated on schema migrations.
//!
//! A module is a dylib exporting one entry point. On reload the engine builds the new
//! build's component tree in a throwaway world, diffs it against the running one, and
//! either swaps the code, applies developer-written migrations, or refuses -- naming the
//! component, both layouts, and the migration that is missing.
//!
//! See `docs/hot-reload.md` for what is detected, what is refused, and what is not caught.

pub mod abi;
pub mod gate;
pub mod host;
pub mod manifest;
pub mod schema;

pub use abi::{ABI_VERSION, AbiToken, ENTRY_SYMBOL, Migration, ModuleDescriptor, ModuleEntry};
pub use flecs_ecs;
pub use gate::{Plan, Refusal, Refused, Verdict};
pub use host::{
    Applied, HotReloader, LoadError, component_instance_count, lookup_component, read_raw,
};
pub use manifest::{Manifest, ModuleManifest};
pub use schema::{ComponentSchema, FieldSchema, Layout, SchemaHash};

/// A field type the migration macro can describe to the gate.
///
/// Implemented only for types flecs' meta addon reflects as primitives. A component field
/// of any other type cannot appear in a declared old layout, which is deliberate: an
/// undescribable field is one the gate could not check, and a check it cannot perform is
/// worse than a compile error.
pub trait ReflectedType {
    /// The path flecs gives this type, e.g. `flecs::meta::u32`.
    const FLECS_PATH: &'static str;
}

macro_rules! reflected {
    ($($ty:ty => $path:literal),* $(,)?) => {
        $(impl ReflectedType for $ty {
            const FLECS_PATH: &'static str = $path;
        })*
    };
}

// The strings are flecs' own symbols for its primitives, which is what the manifest reads
// back out of a live world. They are bare (`u32`), not scoped (`flecs::meta::u32`); using
// the scoped spelling makes every declared migration fail to match the running world.
reflected! {
    bool => "bool",
    char => "char",
    u8 => "u8",
    u16 => "u16",
    u32 => "u32",
    u64 => "u64",
    usize => "uptr",
    i8 => "i8",
    i16 => "i16",
    i32 => "i32",
    i64 => "i64",
    isize => "iptr",
    f32 => "f32",
    f64 => "f64",
}

/// The flecs symbol of a component type, which is its identity across a reload.
///
/// `flecs_ecs` registers a Rust type's symbol as `core::any::type_name`, and unlike the
/// entity path the symbol does not change when a module scopes its components underneath
/// itself.
#[must_use]
pub fn component_symbol<T: ?Sized>() -> String {
    core::any::type_name::<T>().to_owned()
}

#[doc(hidden)]
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "flecs stores sizes and offsets as i32; a component larger than 2GiB is not               a case this engine supports"
)]
pub const fn as_i32(value: usize) -> i32 {
    value as i32
}

/// Declares one migration: the layout the old bytes have, and how to reinterpret them.
///
/// The declared `from` block is checked against what the running world actually reports
/// before a byte is read, so getting it wrong is a refused reload rather than corruption.
///
/// ```ignore
/// hyperion_hot_reload::migration! {
///     component: Health,
///     from: { hp: u32 },
///     with: |old| Health { hp: old.hp as f32 },
/// }
/// ```
#[macro_export]
macro_rules! migration {
    (
        component: $component:ty,
        from: { $($field:ident : $field_ty:ty),* $(,)? },
        with: |$old:ident| $body:expr $(,)?
    ) => {{
        // Declared with the same `repr(Rust)` the old build used, and checked field by
        // field against the running world before it is ever read through.
        #[allow(dead_code, non_snake_case)]
        #[derive(Clone, Copy)]
        struct HotReloadOldLayout {
            $($field: $field_ty),*
        }

        fn apply(old_bytes: &[u8], new_bytes: &mut [u8]) {
            assert_eq!(
                old_bytes.len(),
                ::core::mem::size_of::<HotReloadOldLayout>(),
                "migration input size mismatch"
            );
            assert_eq!(
                new_bytes.len(),
                ::core::mem::size_of::<$component>(),
                "migration output size mismatch"
            );
            // `read_unaligned` because the harvested bytes live in a Vec whose alignment
            // is the allocator's, not the component's.
            let $old: HotReloadOldLayout =
                unsafe { ::core::ptr::read_unaligned(old_bytes.as_ptr().cast()) };
            let produced: $component = $body;
            unsafe {
                ::core::ptr::write_unaligned(new_bytes.as_mut_ptr().cast(), produced);
            }
        }

        $crate::Migration {
            component: $crate::component_symbol::<$component>(),
            from: $crate::ComponentSchema {
                name: $crate::component_symbol::<$component>(),
                size: $crate::as_i32(::core::mem::size_of::<HotReloadOldLayout>()),
                alignment: $crate::as_i32(::core::mem::align_of::<HotReloadOldLayout>()),
                layout: $crate::Layout::Reflected({
                    let mut fields = ::std::vec![
                        $($crate::FieldSchema {
                            name: ::std::string::ToString::to_string(stringify!($field)),
                            type_name: ::std::string::ToString::to_string(
                                <$field_ty as $crate::ReflectedType>::FLECS_PATH,
                            ),
                            offset: $crate::as_i32(
                                ::core::mem::offset_of!(HotReloadOldLayout, $field),
                            ),
                            size: $crate::as_i32(::core::mem::size_of::<$field_ty>()),
                            count: 0,
                        }),*
                    ];
                    fields.sort_by_key(|f: &$crate::FieldSchema| (f.offset, f.name.clone()));
                    fields
                }),
            },
            apply,
        }
    }};
}

//! The structural description of one component, and the rules for comparing two of them.
//!
//! A schema is the unit the reload gate reasons about. Everything the gate can prove is
//! proved from a schema; everything it cannot prove is because a schema said so.

use core::fmt;

/// One primitive leaf of a component, addressed by its absolute byte offset.
///
/// Nested structs are flattened rather than referenced by name. A member recorded only as
/// `inner: probe::Inner @0` would compare equal after `Inner`'s interior changed shape
/// without changing size, and the gate would wave through a reinterpretation that is
/// wrong at every byte past the first field. Flattening makes any interior change show up
/// as a different leaf list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSchema {
    /// Dotted path from the component root, e.g. `inner.a`.
    pub name: String,
    /// Flecs symbol of the leaf's primitive type, e.g. `u32`. Distinguishes `u32` from
    /// `i32` and from `f32`, which comparing sizes alone cannot.
    pub type_name: String,
    /// Absolute offset from the start of the component.
    pub offset: i32,
    pub size: i32,
    /// Inline array length; 0 for a scalar member.
    pub count: i32,
}

/// How much the gate knows about a component's interior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Every leaf reduced to a flecs primitive. This is the only variant the gate can
    /// compare structurally.
    Reflected(Vec<FieldSchema>),
    /// A fieldless enum, described by its constants in declaration order. Catches a
    /// renamed, added, removed or reordered variant.
    Enum(Vec<String>),
    /// Only `size_of` and `align_of` are known. The developer supplies a version number
    /// they bump by hand whenever the interior changes, because nothing else can see it.
    Opaque { declared_version: u32 },
    /// Only `size_of` and `align_of` are known and nobody vouched for the interior.
    Unknown,
}

/// A component as the running world describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentSchema {
    /// The flecs *symbol*, e.g. `demo_module::Health`, which is the Rust `type_name` and
    /// is the identity across a reload.
    ///
    /// Not the flecs path: importing a module scopes its components underneath the module
    /// entity, so `Health` registered by `ArenaModule` has the path
    /// `::demo_module::ArenaModule::Health`, which changes if the module is ever renamed
    /// or re-parented. The symbol does not.
    pub name: String,
    pub size: i32,
    pub alignment: i32,
    pub layout: Layout,
}

/// A 64-bit digest of a schema, printed in refusal messages so a developer can quote the
/// exact old layout a migration must accept.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaHash(pub u64);

impl fmt::Display for SchemaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for SchemaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SchemaHash({self})")
    }
}

/// FNV-1a. Chosen over `DefaultHasher` because the digest is printed to developers and
/// pasted into migration declarations, so it has to be identical across runs and releases.
struct Fnv(u64);

impl Fnv {
    const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x1000_0000_01b3);
        }
    }

    fn write_i32(&mut self, v: i32) {
        self.write(&v.to_le_bytes());
    }

    fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
        self.write(&[0]);
    }
}

impl ComponentSchema {
    /// Digest of everything that determines how the component's bytes are interpreted.
    ///
    /// Deliberately excludes the component name: renaming a component is a rename, not a
    /// layout change, and the two are reported separately.
    #[must_use]
    pub fn hash(&self) -> SchemaHash {
        let mut h = Fnv::new();
        h.write_i32(self.size);
        h.write_i32(self.alignment);
        match &self.layout {
            Layout::Reflected(fields) => {
                h.write_str("reflected");
                for f in fields {
                    h.write_str(&f.name);
                    h.write_str(&f.type_name);
                    h.write_i32(f.offset);
                    h.write_i32(f.size);
                    h.write_i32(f.count);
                }
            }
            Layout::Enum(constants) => {
                h.write_str("enum");
                for c in constants {
                    h.write_str(c);
                }
            }
            Layout::Opaque { declared_version } => {
                h.write_str("opaque");
                h.write(&declared_version.to_le_bytes());
            }
            Layout::Unknown => h.write_str("unknown"),
        }
        SchemaHash(h.0)
    }

    /// Whether the component can hold any bytes at all.
    ///
    /// A zero-sized component is a tag. There is nothing stored to reinterpret, so no
    /// change to it can corrupt data and it never needs a migration.
    #[must_use]
    pub const fn is_zero_sized(&self) -> bool {
        self.size == 0
    }

    /// Whether data stored under `self` can be reinterpreted as `new` with no migration.
    ///
    /// Returns false whenever the answer is not provable, which includes every
    /// [`Layout::Unknown`] component even when its size and alignment are unchanged.
    #[must_use]
    pub fn is_bit_compatible_with(&self, new: &Self) -> bool {
        if self.size != new.size || self.alignment != new.alignment {
            return false;
        }
        if self.is_zero_sized() {
            return true;
        }
        match (&self.layout, &new.layout) {
            (Layout::Reflected(a), Layout::Reflected(b)) => a == b,
            (Layout::Enum(a), Layout::Enum(b)) => a == b,
            (
                Layout::Opaque {
                    declared_version: a,
                },
                Layout::Opaque {
                    declared_version: b,
                },
            ) => a == b,
            // A component that gained or lost reflection has not been proved unchanged,
            // and an unknown interior is never provably unchanged.
            _ => false,
        }
    }

    /// A one-line rendering used in refusal messages.
    #[must_use]
    pub fn describe(&self) -> String {
        let interior = match &self.layout {
            Layout::Reflected(fields) => fields
                .iter()
                .map(|f| format!("{}: {} @{}", f.name, f.type_name, f.offset))
                .collect::<Vec<_>>()
                .join(", "),
            Layout::Enum(constants) => format!("enum {}", constants.join(" | ")),
            Layout::Opaque { declared_version } => {
                format!("<opaque, declared_version = {declared_version}>")
            }
            Layout::Unknown => "<no reflection>".to_owned(),
        };
        format!(
            "{} (size {}, align {}, hash {}) {{ {interior} }}",
            self.name,
            self.size,
            self.alignment,
            self.hash()
        )
    }
}

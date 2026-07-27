//! Decides whether a reload may proceed, before the live world is touched at all.
//!
//! The gate is the reason this is not just a dylib swapper. Its default answer is no: a
//! component crosses a reload only when the engine can prove the old bytes are still
//! readable, or when a developer has written a migration whose declared input matches
//! what the world actually holds.

use core::fmt;

use crate::{
    abi::Migration,
    manifest::ModuleManifest,
    schema::{ComponentSchema, Layout},
};

/// What will happen to one component if the reload proceeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Provably identical layout; the component entity and its data are left alone.
    Keep,
    /// New in this build.
    Added,
    /// Gone from this build. Its data is dropped along with the component entity.
    Dropped,
    /// Layout changed and a matching migration was supplied.
    Migrate {
        old: ComponentSchema,
        new: ComponentSchema,
    },
}

/// Why a reload cannot proceed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Layout changed and nobody said how to reinterpret the old bytes.
    MissingMigration {
        old: ComponentSchema,
        new: ComponentSchema,
    },
    /// A migration exists but describes an old layout the world does not have. Applying it
    /// would read the live bytes at the wrong offsets.
    MigrationDoesNotMatch {
        component: String,
        declared: ComponentSchema,
        actual: ComponentSchema,
    },
    /// The component carries no reflection and nobody declared a version for it, so
    /// "unchanged" is unprovable. Add `#[flecs(meta)]`, or declare an `opaque_versions`
    /// entry and bump it whenever the interior changes.
    UnprovableLayout { component: ComponentSchema },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMigration { old, new } => write!(
                f,
                "component `{}` changed layout and no migration was supplied.\n  was: {}\n  now: \
                 {}\n  Add to the module:\n{}",
                old.name,
                old.describe(),
                new.describe(),
                migration_stub(old)
            ),
            Self::MigrationDoesNotMatch {
                component,
                declared,
                actual,
            } => write!(
                f,
                "migration for `{component}` declares an old layout the running world does not \
                 have, so it would read the live bytes at the wrong offsets.\n  declared: {}\n  \
                 actual:   {}",
                declared.describe(),
                actual.describe()
            ),
            Self::UnprovableLayout { component } => write!(
                f,
                "component `{}` carries no field reflection, so an unchanged layout cannot be \
                 proved and its bytes may not be reinterpreted.\n  {}\n  Add `#[flecs(meta)]` to \
                 the struct, or list it in `opaque_versions` and bump the version whenever its \
                 interior changes.",
                component.name,
                component.describe()
            ),
        }
    }
}

/// `flecs_ecs` registers a module's components as children of the module entity, so the
/// world reports `crate::ArenaModule::Health` where the Rust type path is
/// `crate::Health`. A migration therefore names the component by its leaf.
fn matches_component(declared: &str, world_path: &str) -> bool {
    let leaf = |s: &str| s.rsplit("::").next().unwrap_or(s).to_owned();
    leaf(declared) == leaf(world_path)
}

/// A copy-pasteable starting point for the migration the developer has to write.
fn migration_stub(old: &ComponentSchema) -> String {
    let short = old.name.rsplit("::").next().unwrap_or(&old.name);
    let fields = match &old.layout {
        Layout::Reflected(fields) => fields
            .iter()
            .map(|f| {
                format!(
                    "            {}: {},",
                    f.name,
                    f.type_name.rsplit("::").next().unwrap_or(&f.type_name)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Layout::Enum(constants) => format!("            // was enum {}", constants.join(" | ")),
        Layout::Opaque { .. } | Layout::Unknown => {
            "            // no reflection: declare the old fields by hand".to_owned()
        }
    };
    format!(
        "    hyperion_hot_reload::migration! {{\n        component: {short},\n        from: \
         {{\n{fields}\n        }},\n        with: |old| {short} {{ /* map each field */ }},\n    \
         }}"
    )
}

/// The accepted outcome of gating one reload.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Components to migrate, in the order they will be processed.
    pub migrations: Vec<PlannedMigration>,
    /// Reported for visibility; these keep their data untouched.
    pub kept: Vec<String>,
    pub added: Vec<String>,
    pub dropped: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PlannedMigration {
    pub component: String,
    pub old: ComponentSchema,
    pub new: ComponentSchema,
    /// Index into the module's migration list.
    pub migration_index: usize,
}

/// The reload was refused. Nothing has been changed.
#[derive(Clone, Debug)]
pub struct Refused {
    pub module: String,
    pub reasons: Vec<Refusal>,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "refusing to reload module `{}`: {} unmigrated schema change(s). The running world \
             was not modified.",
            self.module,
            self.reasons.len()
        )?;
        for (i, r) in self.reasons.iter().enumerate() {
            writeln!(f, "\n[{}] {r}", i + 1)?;
        }
        Ok(())
    }
}

impl std::error::Error for Refused {}

/// Compares the manifest of the running module against the manifest the new build
/// produced in a probe world, and decides.
///
/// `has_instances` answers "does the live world currently hold any of this component";
/// a component with no stored data cannot be corrupted, so it is never a reason to refuse.
pub fn plan(
    old: Option<&ModuleManifest>,
    new: &ModuleManifest,
    migrations: &[Migration],
    has_instances: &dyn Fn(&str) -> bool,
) -> Result<Plan, Refused> {
    let mut out = Plan::default();
    let mut reasons = Vec::new();
    let empty = Vec::new();
    let old_components = old.map_or(&empty, |m| &m.components);

    for new_schema in &new.components {
        let Some(old_schema) = old_components.iter().find(|c| c.name == new_schema.name) else {
            out.added.push(new_schema.name.clone());
            continue;
        };

        if old_schema.is_bit_compatible_with(new_schema) {
            out.kept.push(new_schema.name.clone());
            continue;
        }

        // A tag that grew fields has no stored bytes to misinterpret: the old component
        // occupied zero bytes per entity, so there is nothing a migration could read.
        if old_schema.is_zero_sized() {
            out.kept.push(new_schema.name.clone());
            continue;
        }

        // An unprovable layout is only dangerous once something is stored in it.
        if !has_instances(&old_schema.name) {
            out.kept.push(new_schema.name.clone());
            continue;
        }

        if matches!(old_schema.layout, Layout::Unknown)
            || matches!(new_schema.layout, Layout::Unknown)
        {
            reasons.push(Refusal::UnprovableLayout {
                component: new_schema.clone(),
            });
            continue;
        }

        match migrations
            .iter()
            .position(|m| matches_component(&m.component, &new_schema.name))
        {
            Some(index) => {
                let declared = &migrations[index].from;
                if declared.size != old_schema.size
                    || declared.alignment != old_schema.alignment
                    || declared.layout != old_schema.layout
                {
                    reasons.push(Refusal::MigrationDoesNotMatch {
                        component: new_schema.name.clone(),
                        declared: declared.clone(),
                        actual: old_schema.clone(),
                    });
                } else {
                    out.migrations.push(PlannedMigration {
                        component: new_schema.name.clone(),
                        old: old_schema.clone(),
                        new: new_schema.clone(),
                        migration_index: index,
                    });
                }
            }
            None => reasons.push(Refusal::MissingMigration {
                old: old_schema.clone(),
                new: new_schema.clone(),
            }),
        }
    }

    for old_schema in old_components {
        if !new.components.iter().any(|c| c.name == old_schema.name) {
            out.dropped.push(old_schema.name.clone());
        }
    }

    if reasons.is_empty() {
        Ok(out)
    } else {
        Err(Refused {
            module: new.name.clone(),
            reasons,
        })
    }
}

//! Groups an operator assigns in the server's configuration.
//!
//! # The problem this solves
//!
//! [`PermissionStorage`] starts empty and is only ever written by `/perms set`,
//! which is itself an `Admin` command. A server booted against a fresh database
//! therefore has no administrator and no way to appoint one: the only door to
//! the group is behind the group. Before [`SeededGroups`] the way through that
//! was that `/perms set` was mistakenly gated at `Normal`, so anybody could
//! appoint themselves (ENG-10871).
//!
//! So an operator names the accounts that hold a group, in configuration,
//! before the server starts. This is the ordinary way to run the server with
//! administrators, not a test affordance: `docker run -e HYPERION_PERMISSIONS=...`
//! and a `.env` file both reach it, and it is the only path to `Admin` on a
//! database that has never had one.
//!
//! # Configuration
//!
//! `HYPERION_PERMISSIONS` is a comma separated list of `<uuid>=<group>`:
//!
//! ```text
//! HYPERION_PERMISSIONS=069a79f4-44e9-4726-a5be-fca90e38aaf5=Admin,853c80ef-3c37-49fd-aa49-938b674adae6=Moderator
//! ```
//!
//! The group names are the variants of [`Group`], matched without regard to
//! case. A malformed entry stops the server at startup rather than being
//! skipped, because the failure mode of a silently ignored line is a server
//! that boots with nobody in charge and no indication why.
//!
//! Configuration wins over the database in both directions. A player named here
//! joins in the group named here whatever `/perms set` did to them last week,
//! and their group is not written back when they leave, so deleting a line here
//! is enough to take the group away. A player not named here is unaffected and
//! keeps whatever the database holds.
//!
//! # What a seeded group is worth
//!
//! Exactly as much as the server's idea of who a player is. This server is
//! offline-mode: `hyperion::net::protocol` mints a random profile id for a
//! client that presents a zero one, and takes the client's word for a non-zero
//! one. Nothing verifies it against Mojang. So on an offline-mode server a
//! profile id is a claim, and any client can make any claim. That is a property
//! of offline mode rather than of this file, and it is the reason to key on the
//! profile id rather than on the username anyway: when the server does
//! authenticate, the id is the thing Mojang signed.
//!
//! [`PermissionStorage`]: crate::storage::PermissionStorage

use std::{collections::HashMap, env, str::FromStr};

use anyhow::{Context, bail};
use clap::ValueEnum;
use flecs_ecs::macros::Component;

use crate::Group;

/// The environment variable read at startup.
pub const ENV_VAR: &str = "HYPERION_PERMISSIONS";

/// The groups configuration assigns, by profile id.
///
/// A singleton, set by [`crate::PermissionModule`] from [`ENV_VAR`]. An
/// embedder that gets its configuration from somewhere else can overwrite it
/// after importing the module.
#[derive(Component, Default, Debug, Clone, PartialEq, Eq)]
pub struct SeededGroups(HashMap<uuid::Uuid, Group>);

impl SeededGroups {
    /// The group configuration assigns to `uuid`, if it assigns one.
    #[must_use]
    pub fn get(&self, uuid: uuid::Uuid) -> Option<Group> {
        self.0.get(&uuid).copied()
    }

    /// Whether configuration says anything about `uuid`.
    #[must_use]
    pub fn contains(&self, uuid: uuid::Uuid) -> bool {
        self.0.contains_key(&uuid)
    }

    /// How many accounts configuration names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether configuration names nobody, which is the default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Read [`ENV_VAR`], or nobody when it is unset or empty.
    ///
    /// # Errors
    /// When the variable is set to something that is not a list of
    /// `<uuid>=<group>`, naming the entry that could not be read.
    pub fn from_env() -> anyhow::Result<Self> {
        match env::var(ENV_VAR) {
            Ok(value) => value.parse(),
            Err(env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("reading {ENV_VAR} from the environment"))
            }
        }
    }
}

impl FromStr for SeededGroups {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        let mut seeded = HashMap::new();

        for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let Some((uuid, group)) = entry.split_once('=') else {
                bail!(
                    "{ENV_VAR} entry {entry:?} is not <uuid>=<group>; it is a comma separated \
                     list such as 069a79f4-44e9-4726-a5be-fca90e38aaf5=Admin"
                );
            };

            let uuid = uuid::Uuid::parse_str(uuid.trim())
                .with_context(|| format!("{ENV_VAR} entry {entry:?} does not start with a uuid"))?;

            let group = group.trim();
            let group = Group::from_str(group, true).map_err(|_| {
                // Listed from the enum rather than written out, so a group
                // added or renamed cannot leave this message describing the
                // groups of a year ago.
                let known: Vec<String> = Group::value_variants()
                    .iter()
                    .filter_map(ValueEnum::to_possible_value)
                    .map(|value| value.get_name().to_owned())
                    .collect();
                anyhow::anyhow!(
                    "{ENV_VAR} entry {entry:?} names no group; the groups are {known:?}"
                )
            })?;

            if let Some(previous) = seeded.insert(uuid, group) {
                bail!("{ENV_VAR} names {uuid} twice, as {previous:?} and as {group:?}");
            }
        }

        Ok(Self(seeded))
    }
}

#[cfg(test)]
mod tests {
    use super::{Group, SeededGroups};

    #[test]
    fn an_unset_variable_names_nobody() {
        assert!("".parse::<SeededGroups>().unwrap().is_empty());
    }

    #[test]
    fn a_list_is_read_whitespace_and_all() {
        let seeded: SeededGroups = " 069a79f4-44e9-4726-a5be-fca90e38aaf5 = Admin , \
                                    853c80ef-3c37-49fd-aa49-938b674adae6=moderator"
            .parse()
            .unwrap();

        assert_eq!(seeded.len(), 2);
        assert_eq!(
            seeded.get("069a79f4-44e9-4726-a5be-fca90e38aaf5".parse().unwrap()),
            Some(Group::Admin)
        );
        assert_eq!(
            seeded.get("853c80ef-3c37-49fd-aa49-938b674adae6".parse().unwrap()),
            Some(Group::Moderator)
        );
    }

    /// The failure this file exists to make loud. A skipped line is a server
    /// that boots with nobody in charge and no way to tell why.
    #[test]
    fn a_typo_is_an_error_rather_than_a_skipped_line() {
        for broken in [
            "069a79f4-44e9-4726-a5be-fca90e38aaf5",
            "069a79f4-44e9-4726-a5be-fca90e38aaf5=Owner",
            "andrew=Admin",
            "069a79f4-44e9-4726-a5be-fca90e38aaf5=Admin,\
             069a79f4-44e9-4726-a5be-fca90e38aaf5=Normal",
        ] {
            assert!(
                broken.parse::<SeededGroups>().is_err(),
                "{broken:?} was accepted"
            );
        }
    }
}

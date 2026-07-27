//! Kits are prefabs, and a kit is data.
//!
//! A kit module builds one prefab carrying the kit's stats and a child prefab
//! per ability, then never runs again. Choosing a kit instantiates the prefab
//! onto the player, which copies the stats and creates that player's own
//! ability entities with their own cooldowns.
//!
//! Nothing in this file, or in any other subsystem, knows the name of a kit.
//! That is the whole claim: [`crate::module::kits`] is an import list, and the
//! test in `tests/modularity.rs` adds a kit from outside the crate to prove it.

use flecs_ecs::prelude::*;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{
            Ability, Cast, ChargeTime, Cooldown, CooldownSpec, Description, EnergyCost, Grants,
            Item, Named, OnActivate, OnRelease, RequiresGround, Slot,
        },
        damage::Armor,
        knockback::KnockbackTaken,
        player::{Energy, Health, JumpsLeft},
    },
    server::HotbarItem,
};

/// Tag on kit prefabs.
#[derive(Component, Debug)]
pub struct Kit;

/// Relationship: `(Playing, kit)` on a player.
///
/// Exclusive — you play exactly one kit — which means selecting a new kit in
/// the lobby removes the old edge for free instead of needing a clear-then-set
/// pair of operations that can be interrupted halfway.
#[derive(Component, Debug)]
pub struct Playing;

/// The four numbers Mineplex tuned every kit on, plus the ones the engine needs.
///
/// Mineplex loaded these from a Google Sheet at runtime rather than compiling
/// them in, which is why the leaked source documents the mechanism but not the
/// values. See `docs/smash-design.md` for where each number came from.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct KitStats {
    /// Flat melee damage. Mineplex replaced the weapon's damage entirely with
    /// this, so a kit hits for the same amount with any item, including a fist.
    pub melee_damage: f32,
    /// Vanilla armour points. Reduction is `points * 4%`.
    pub armor: f32,
    /// Knockback taken, as a multiplier. 1.25 is the wiki's "125%".
    pub knockback_taken: f32,
    /// Health per second, out of combat and in.
    pub regen: f32,
    /// Seconds between losing half a hunger shank.
    pub hunger_interval: f32,
    pub max_health: f32,
    /// Impulse of the double jump.
    pub jump_power: f32,
    /// Whether the double jump goes where you look (Wolf, Spider) or straight
    /// up (everyone else).
    pub jump_control: bool,
    /// Present only on kits with an energy bar.
    pub energy: Option<(f32, f32)>,
}

impl Default for KitStats {
    fn default() -> Self {
        Self {
            melee_damage: 5.0,
            armor: 10.0,
            knockback_taken: 1.0,
            regen: 0.25,
            hunger_interval: 7.75,
            max_health: 20.0,
            jump_power: 1.0,
            jump_control: false,
            energy: None,
        }
    }
}

/// Human-readable kit name, and the key the lobby selects on.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct KitName(pub &'static str);

/// Gem cost. Zero means a free kit.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct KitCost(pub u32);

/// One-line pitch, shown in the kit menu.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct KitBlurb(pub &'static str);

/// Marks the ability the Smash Crystal unlocks, rather than one of the four the
/// kit starts with.
#[derive(Component, Debug)]
pub struct Ultimate;

const fn noop(_: &Cast<'_>) {}

/// One ability, as a kit file declares it.
///
/// Written to be filled in with struct update syntax so a kit file reads as a
/// list of the things that ability actually has, rather than a wall of
/// defaults:
///
/// ```ignore
/// AbilitySpec {
///     name: "Blink",
///     item: "minecraft:iron_axe",
///     slot: 1,
///     cooldown: 7.0,
///     activate: blink,
///     ..AbilitySpec::DEFAULT
/// }
/// ```
#[derive(Debug, Copy, Clone)]
pub struct AbilitySpec {
    pub name: &'static str,
    pub item: &'static str,
    pub slot: u8,
    pub description: &'static str,
    pub cooldown: f32,
    /// Seconds of holding to reach full charge. `Some` makes this a
    /// hold-and-release ability and routes it to `activate` on release with the
    /// charge fraction filled in.
    pub charge_time: Option<f32>,
    pub energy_cost: Option<f32>,
    pub requires_ground: bool,
    pub activate: fn(&Cast<'_>),
}

impl AbilitySpec {
    pub const DEFAULT: Self = Self {
        name: "",
        item: "minecraft:stick",
        slot: 0,
        description: "",
        cooldown: 0.0,
        charge_time: None,
        energy_cost: None,
        requires_ground: false,
        activate: noop,
    };
}

impl Default for AbilitySpec {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Builds one kit prefab. Returned by [`define`].
pub struct KitBuilder<'w> {
    world: &'w World,
    kit: EntityView<'w>,
}

/// Start a kit definition. Call from a kit module's [`Module::module`].
#[must_use]
pub fn define<'w>(world: &'w World, name: &'static str, stats: KitStats) -> KitBuilder<'w> {
    let kit = world
        .prefab_named(name)
        .add(Kit::id())
        .set(KitName(name))
        .set(stats);
    KitBuilder { world, kit }
}

impl<'w> KitBuilder<'w> {
    #[must_use]
    pub fn cost(self, gems: u32) -> Self {
        self.kit.set(KitCost(gems));
        self
    }

    #[must_use]
    pub fn blurb(self, text: &'static str) -> Self {
        self.kit.set(KitBlurb(text));
        self
    }

    /// Add one of the kit's starting abilities.
    #[must_use]
    pub fn ability(self, spec: AbilitySpec) -> Self {
        self.build_ability(spec, false);
        self
    }

    /// Add the Smash Crystal ability. Not granted at spawn; the crystal grants
    /// it and its expiry takes it back.
    #[must_use]
    pub fn ultimate(self, spec: AbilitySpec) -> Self {
        let ability = self.build_ability(spec, true);
        ability.add(Ultimate::id());
        self
    }

    /// Finish the definition. Kit modules end on this.
    pub const fn register(self) {}

    /// The kit prefab, for a caller that wants to decorate it further.
    #[must_use]
    pub const fn prefab(&self) -> EntityView<'w> {
        self.kit
    }

    fn build_ability(&self, spec: AbilitySpec, ultimate: bool) -> EntityView<'w> {
        let ability = self
            .world
            .prefab_named(spec.name)
            .add(Ability::id())
            .set(Named(spec.name))
            .set(Item(spec.item))
            .set(Slot(spec.slot))
            .set(Description(spec.description))
            .set(CooldownSpec(spec.cooldown))
            .set(Cooldown::default());

        if let Some(charge) = spec.charge_time {
            ability.set(ChargeTime(charge));
            ability.set(OnRelease(spec.activate));
        } else {
            ability.set(OnActivate(spec.activate));
        }
        if let Some(cost) = spec.energy_cost {
            ability.set(EnergyCost(cost));
        }
        if spec.requires_ground {
            ability.add(RequiresGround::id());
        }
        if !ultimate {
            self.kit.add((Grants, ability));
        }
        ability
    }
}

/// Put `kit` on `player`: copy the stats, give them their own ability entities.
///
/// The ability entities are fresh instances rather than the kit's prefabs
/// because cooldowns are per player. Instantiating a prefab in flecs copies its
/// components by default, so each instance starts with its own zeroed
/// [`Cooldown`] and the shared static data comes along for free.
pub fn apply(world: &World, player: EntityView<'_>, kit: EntityView<'_>) {
    let Some(stats) = kit.try_get::<&KitStats>(|s| *s) else {
        return;
    };

    revoke(player);

    player
        .add((Playing, kit))
        .set(Armor(stats.armor))
        .set(KnockbackTaken(stats.knockback_taken))
        .set(Health::full(stats.max_health))
        .set(JumpsLeft(1));

    if let Some((max, regen)) = stats.energy {
        player.set(Energy::full(max, regen));
    } else {
        player.remove(Energy::id());
    }

    let mut prefabs = Vec::new();
    kit.each_target_view(Grants, |ability| prefabs.push(ability.id()));
    for prefab in prefabs {
        let instance = world.entity().is_a(prefab).child_of(player);
        player.add((Grants, instance));
    }
}

/// Strip whatever kit a player is currently carrying.
pub fn revoke(player: EntityView<'_>) {
    let mut granted = Vec::new();
    player.each_target_view(Grants, |ability| granted.push(ability.id()));
    for ability in granted {
        player.remove((Grants, ability));
        player.world().entity_from_id(ability).destruct();
    }
}

/// The hotbar a kit's abilities imply. The lobby and respawn both push this.
#[must_use]
pub fn hotbar(player: EntityView<'_>) -> Vec<HotbarItem> {
    let mut items = Vec::new();
    player.each_target_view(Grants, |ability| {
        let (Some(slot), Some(item), Some(name)) = (
            ability.try_get::<&Slot>(|s| s.0),
            ability.try_get::<&Item>(|i| i.0),
            ability.try_get::<&Named>(|n| n.0),
        ) else {
            return;
        };
        let lore = ability
            .try_get::<&Description>(|d| d.0)
            .filter(|d| !d.is_empty())
            .map(|d| vec![d.to_owned()])
            .unwrap_or_default();
        items.push(HotbarItem {
            slot,
            item,
            name: name.to_owned(),
            lore,
        });
    });
    items.sort_by_key(|item| item.slot);
    items
}

/// Every registered kit, in registration order. A query, not a list someone has
/// to remember to append to.
#[must_use]
pub fn registry(world: &World) -> Vec<Entity> {
    let mut kits = Vec::new();
    world
        .query::<()>()
        .with(Kit::id())
        .with(id::<flecs::Prefab>())
        .build()
        .each_entity(|entity, ()| kits.push(entity.id()));
    kits
}

/// Look a kit up by its [`KitName`].
///
/// Deliberately not `world.try_lookup(name)`: a kit prefab is created inside
/// its own module's scope, so its path is `smash::kits::Skeleton::Skeleton` and
/// a root lookup of "Skeleton" misses it. Matching on the component instead
/// means a kit's registered name is independent of where its module chose to
/// live, which is one less thing a kit author can get wrong.
#[must_use]
pub fn by_name<'w>(world: &'w World, name: &str) -> Option<EntityView<'w>> {
    let mut found: Option<Entity> = None;
    world
        .query::<&KitName>()
        .with(id::<flecs::Prefab>())
        .build()
        .each_entity(|entity, kit_name| {
            if found.is_none() && kit_name.0 == name {
                found = Some(entity.id());
            }
        });
    found.map(|id| world.entity_from_id(id))
}

#[derive(Component)]
pub struct KitModule;

impl Module for KitModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Kit");

        world.component::<Kit>();
        world.component::<KitStats>();
        world.component::<KitName>();
        world.component::<KitCost>();
        world.component::<KitBlurb>();
        world.component::<Ultimate>();
        world.component::<Playing>().add(flecs::Exclusive);
    }
}

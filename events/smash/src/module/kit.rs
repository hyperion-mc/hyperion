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
use hyperion::simulation::command::SuggestionLabel;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{
            Ability, Cast, ChargeTime, Cooldown, CooldownSpec, Description, EnergyCost, GrantedFor,
            Grants, Item, Named, Observable, OnActivate, OnRelease, Proves, RefundsOnHit,
            RequiresGround, Slot,
        },
        damage::Armor,
        knockback::KnockbackTaken,
        player::{Energy, Health, JumpsLeft},
        sound::{self, Levels, PlaysOnCast, PlaysOnDeath, PlaysOnHurt, PlaysOnSelect},
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

/// The skin the kit's mob wears, as a Mojang-signed profile property.
///
/// On the kit prefab and not on the player. Which mob you are is the relation
/// `(Playing, kit)`, so the look belongs to the thing that edge points at, and
/// a player wears it for exactly as long as the edge exists. Nothing copies it
/// into a field on the player except the one adapter system that has to hand
/// it to a host that speaks profiles.
///
/// Signed, and not merely present, because that is what the client enforces.
/// `SkinManager.createLookup` filters on `!requireSecure || skin.secure()`, and
/// `PlayerInfo.createSkinLookup` passes `requireSecure = !isLocalPlayer`, so an
/// unsigned `textures` property dresses you up for yourself alone and leaves
/// everyone else looking at Steve. See `skins/README.md`.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct KitSkin {
    /// Base64 of the Mojang textures payload: the `value` of the property.
    pub textures: &'static str,
    /// Base64 RSA-SHA1 signature over `textures`, by Mojang's profile property
    /// key.
    pub signature: &'static str,
}

/// Declare a kit's skin from the pair of files in `events/smash/skins/`.
///
/// A macro rather than two [`include_str!`] calls at each call site, so a kit
/// file names its mob once and cannot pair one mob's payload with another's
/// signature. Paths are anchored at `CARGO_MANIFEST_DIR` rather than written
/// relative to the calling file, so moving a kit module does not silently
/// break its skin.
#[macro_export]
macro_rules! kit_skin {
    ($mob:literal) => {
        $crate::module::kit::KitSkin {
            textures: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/skins/",
                $mob,
                ".value"
            )),
            signature: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/skins/", $mob, ".sig")),
        }
    };
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

/// The mob that stands on this kit's podium in the lobby.
///
/// A vanilla entity id, the same vocabulary [`AbilitySpec::item`] uses for
/// items. It is the kit's own to declare for the same reason its abilities
/// are: nothing outside a kit file may name a kit, so a selector that mapped
/// kit to mob centrally would be the one table this crate has spent its whole
/// design avoiding.
///
/// Mostly the obvious thing, because the roster is named after the mobs. The
/// two that are not are the ones whose kit name is Mineplex's rather than
/// Mojang's: the Sky Squid is a `minecraft:squid` and the Snowman is a
/// `minecraft:snow_golem`.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct KitMob(pub &'static str);

/// What a kit that never called [`KitBuilder::mob`] stands on its podium.
///
/// A podium still appears, because a kit nobody can select is worse than an
/// ugly one, and `tests/selector.rs` fails on any registered kit that leaves it
/// at this.
pub const DEFAULT_MOB: &str = "minecraft:armor_stand";

/// Marks the ability the Smash Crystal unlocks, rather than one of the four the
/// kit starts with.
#[derive(Component, Debug)]
pub struct Ultimate;

/// How many keys a player has. The hotbar is nine slots and there is no tenth.
pub const HOTBAR_SLOTS: u8 = 9;

/// Where every kit's Smash Crystal ability goes: the far right of the bar.
///
/// Mineplex put the ultimate at the end and the kit's own weapon under the hand,
/// and the whole roster agreed on this end of the bar before it agreed on the
/// other. Named here rather than repeated as an `8` in fifteen kit files,
/// because a constant is the thing a reader can look up.
pub const ULTIMATE_SLOT: u8 = HOTBAR_SLOTS - 1;

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
///     cooldown: 7.0,
///     proves: &[Observable::TeleportsCaster],
///     sound: "minecraft:entity.enderman.teleport",
///     activate: blink,
///     ..AbilitySpec::DEFAULT
/// }
/// ```
///
/// `proves` and `sound` are the two fields with no sensible default. Everything else
/// describes the ability; that field is the ability's own claim about what a
/// player will see, and it is what the two gates enumerate. Leaving it empty is
/// caught by `tests/abilities.rs`, so a kit cannot be added without saying what
/// its abilities do.
#[derive(Debug, Copy, Clone)]
pub struct AbilitySpec {
    pub name: &'static str,
    pub item: &'static str,
    pub description: &'static str,
    pub cooldown: f32,
    /// Seconds of holding to reach full charge. `Some` makes this a
    /// hold-and-release ability and routes it to `activate` on release with the
    /// charge fraction filled in.
    pub charge_time: Option<f32>,
    pub energy_cost: Option<f32>,
    pub requires_ground: bool,
    /// Landing a hit clears the cooldown instead of the clock doing it.
    ///
    /// Chicken Missile is the only ability in the roster built this way, and the
    /// wiki calls it the kit's strongest point. It is declared rather than left
    /// implicit because "a cooldown refuses the next use" is otherwise a rule
    /// every ability is held to, and one that is meant to be broken has to say
    /// so where the gates can read it.
    pub refunds_on_hit: bool,
    /// What a client sees when this fires. Must not be empty.
    pub proves: &'static [Observable],
    /// The vanilla sound event firing this plays, e.g.
    /// `minecraft:entity.blaze.shoot`. Must not be empty, and must be a sound
    /// the client already owns: `tests/sound.rs` enumerates the whole roster
    /// and holds every id against the generated `minecraft:sound_event`
    /// registry, so an ability that forgets one fails there rather than going
    /// quietly silent in play.
    pub sound: &'static str,
    pub activate: fn(&Cast<'_>),
}

impl AbilitySpec {
    pub const DEFAULT: Self = Self {
        name: "",
        item: "minecraft:stick",
        description: "",
        cooldown: 0.0,
        charge_time: None,
        energy_cost: None,
        requires_ground: false,
        refunds_on_hit: false,
        proves: &[],
        sound: "",
        activate: noop,
    };
}

impl Default for AbilitySpec {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How loud an ultimate's cast is, against 1.0 for an ordinary ability.
pub const ULTIMATE_VOLUME: f32 = 1.6;

/// The voice of the mob a kit is.
///
/// Three sounds, and what is deliberately *not* among them is the noise a
/// landed hit makes: that one is the same for every kit so that its pitch and
/// volume can mean how hard rather than who. See
/// [`crate::module::sound::IMPACT`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct KitSounds {
    /// Played to the player who just picked this mob off its podium, and to
    /// nobody else. See [`crate::module::sound::PlaysOnSelect`].
    ///
    /// The mob's own greeting rather than a menu click: what a player has just
    /// done is choose to *be* a skeleton, and the skeleton rattling back is
    /// what says so without a line of text. Every one of these is the mob's
    /// vanilla ambient sound where it has one, and `tests/sound.rs` holds all
    /// fifteen against the generated `minecraft:sound_event` registry and
    /// against each other, because a roster where two mobs answer alike is a
    /// roster where the sound has stopped carrying which mob it was.
    pub select: &'static str,
    /// Played on the victim when they are hurt, whoever hurt them.
    pub hurt: &'static str,
    /// Played where they died.
    pub death: &'static str,
}

/// Builds one kit prefab. Returned by [`define`].
pub struct KitBuilder<'w> {
    world: &'w World,
    kit: EntityView<'w>,
    /// The slot the next [`KitBuilder::ability`] call takes.
    ///
    /// The layout is a property of the kit, so the kit is what owns it. A
    /// per-ability `slot: u8` was the same fact written fifty-one times, and
    /// twelve of the fifteen kits wrote it starting from 1: every one of them
    /// left the slot a player's hand rests on at spawn empty, which is a kit
    /// whose first ability cannot be fired without pressing a key first.
    next_slot: u8,
}

/// Start a kit definition. Call from a kit module's [`Module::module`].
#[must_use]
pub fn define<'w>(world: &'w World, name: &'static str, stats: KitStats) -> KitBuilder<'w> {
    let kit = world
        .prefab_named(name)
        .add(Kit::id())
        .set(KitName(name))
        .set(stats);
    KitBuilder {
        world,
        kit,
        next_slot: 0,
    }
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

    /// What this kit's mob sounds like when it is chosen, when it is hurt and
    /// when it dies.
    ///
    /// Hung off the kit prefab as `(PlaysOnSelect, sound)`,
    /// `(PlaysOnHurt, sound)` and `(PlaysOnDeath, sound)`, so the selector, the
    /// damage path and the death path each reach it through the player's own
    /// `(Playing, kit)` edge and no subsystem learns a kit name to do it.
    #[must_use]
    pub fn sounds(self, sounds: KitSounds) -> Self {
        let voice = Levels::default();
        self.kit.add((
            PlaysOnSelect,
            sound::intern(self.world, sounds.select, voice),
        ));
        self.kit
            .add((PlaysOnHurt, sound::intern(self.world, sounds.hurt, voice)));
        self.kit
            .add((PlaysOnDeath, sound::intern(self.world, sounds.death, voice)));
        self
    }

    /// The mob that stands on this kit's podium. See [`KitMob`].
    #[must_use]
    pub fn mob(self, entity: &'static str) -> Self {
        self.kit.set(KitMob(entity));
        self
    }

    /// Give the kit its mob's skin. Write it as `kit_skin!("zombie")`.
    #[must_use]
    pub fn skin(self, skin: KitSkin) -> Self {
        self.kit.set(skin);
        self
    }

    /// Add one of the kit's starting abilities, in the next slot along.
    ///
    /// The first goes in slot 0, which is where a player's hand rests when they
    /// spawn, and each one after it takes the key to its right. The order a kit
    /// file declares its abilities in *is* the layout, so there is no number to
    /// get wrong and no way to leave a hole in the bar.
    ///
    /// # Panics
    ///
    /// If a kit declares more starting abilities than fit to the left of
    /// [`ULTIMATE_SLOT`]. The next one would land on the ultimate's key and one
    /// of the two would be unreachable, so this refuses at startup rather than
    /// shipping a kit with an ability nobody can press.
    #[must_use]
    pub fn ability(mut self, spec: AbilitySpec) -> Self {
        let slot = self.next_slot;
        assert!(
            slot < ULTIMATE_SLOT,
            "{} declares more than {ULTIMATE_SLOT} starting abilities, so this one would land on \
             the Smash Crystal's key",
            spec.name,
        );
        self.build_ability(spec, slot, false);
        self.next_slot = slot + 1;
        self
    }

    /// Add the Smash Crystal ability. Not granted at spawn; the crystal grants
    /// it and its expiry takes it back.
    ///
    /// Always [`ULTIMATE_SLOT`], whatever else the kit declares: it is the one
    /// binding a player carries across every mob they play.
    #[must_use]
    pub fn ultimate(self, spec: AbilitySpec) -> Self {
        self.build_ability(spec, ULTIMATE_SLOT, true);
        self
    }

    /// Finish the definition. Kit modules end on this.
    pub const fn register(self) {}

    /// The kit prefab, for a caller that wants to decorate it further.
    #[must_use]
    pub const fn prefab(&self) -> EntityView<'w> {
        self.kit
    }

    fn build_ability(&self, spec: AbilitySpec, slot: u8, ultimate: bool) -> EntityView<'w> {
        let ability = self
            .world
            .prefab_named(spec.name)
            .add(Ability::id())
            .set(Named(spec.name))
            .set(Item(spec.item))
            .set(Slot(slot))
            .set(Description(spec.description))
            .set(CooldownSpec(spec.cooldown))
            .set(Proves(spec.proves))
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
        if spec.refunds_on_hit {
            ability.add(RefundsOnHit::id());
        }
        if ultimate {
            ability.add(Ultimate::id());
        }
        // On the ability entity, not in a table somewhere keyed by its name.
        // What a player fires is an instance of this prefab, and how the
        // declaration reaches that instance is `module/sound.rs`'s business.
        if !spec.sound.is_empty() {
            let sound = sound::intern(self.world, spec.sound, Levels {
                // An ultimate carries further, and volume is range: see
                // `hyperion::net::agnostic::RANGE_PER_VOLUME`. A Smash Crystal
                // going off is the loudest thing in a match and should be heard
                // by people who are not in it yet. Everything else about how it
                // plays is `Levels::default`, which is where the reasoning for
                // the category lives.
                volume: if ultimate { ULTIMATE_VOLUME } else { 1.0 },
                ..Levels::default()
            });
            ability.add((PlaysOnCast, sound));
        }
        // Every ability the kit has, ultimate included, hangs off the same
        // relationship, so one traversal enumerates the whole kit. What
        // separates them is the `Ultimate` tag, which `apply` filters on: an
        // ultimate that was reachable only through its flecs scope path was an
        // ability no registry could see and therefore no gate could test.
        self.kit.add((Grants, ability));
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
    kit.each_target_view(Grants, |ability| {
        // The ultimate is the Smash Crystal's to hand out, so it is on the kit
        // and not on the player until one is picked up.
        if !ability.has(Ultimate::id()) {
            prefabs.push(ability.id());
        }
    });
    for prefab in prefabs {
        let instance = world.entity().is_a(prefab).child_of(player);
        player.add((Grants, instance));
    }
}

/// Hand `player` their kit's Smash Crystal ability for `seconds`.
///
/// The window is the ability layer's ([`crate::module::ability::GrantedFor`]
/// counts it down and takes the ability back); what spawns a crystal in the
/// arena for somebody to walk into is the arena's, and does not exist yet. Until
/// it does this is the whole of the mechanic, and `/crystal` is the way a player
/// or a test reaches it.
///
/// Returns `false` when the player has no kit, the kit declares no ultimate, or
/// they are already holding one.
#[must_use]
pub fn grant_ultimate(world: &World, player: EntityView<'_>, seconds: f32) -> bool {
    let Some(kit) = player.find_target(Playing, |_| true) else {
        return false;
    };
    let Some(prefab) = kit.find_target(Grants, |ability| ability.has(Ultimate::id())) else {
        return false;
    };
    if player
        .find_target(Grants, |ability| ability.has(Ultimate::id()))
        .is_some()
    {
        return false;
    }

    let instance = world.entity().is_a(prefab).child_of(player);
    instance.set(GrantedFor { remaining: seconds });
    player.add((Grants, instance));
    true
}

/// The skin of the mob `player` is currently playing as.
///
/// Read through `(Playing, kit)` on every call rather than cached on the
/// player, so a kit change cannot leave a stale look behind: there is only one
/// copy of the answer and it lives on the kit.
#[must_use]
pub fn skin_of(player: EntityView<'_>) -> Option<KitSkin> {
    player
        .find_target(Playing, |_| true)?
        .try_get::<&KitSkin>(|skin| *skin)
}

/// The name of the ultimate `player`'s kit declares, whether or not they hold
/// it. `None` for a player with no kit.
#[must_use]
pub fn ultimate_name(player: EntityView<'_>) -> Option<&'static str> {
    player
        .find_target(Playing, |_| true)?
        .find_target(Grants, |ability| ability.has(Ultimate::id()))?
        .try_get::<&Named>(|name| name.0)
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

/// Who is playing what, right now.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Claim {
    pub kit: Entity,
    pub player: Entity,
}

/// Every kit somebody is currently playing.
///
/// Derived on every call by walking the `(Playing, kit)` edges themselves.
/// There is deliberately no `taken` flag on a kit and no set of claims kept on
/// the side, because a second copy of this answer is a thing that can disagree
/// with the first, and the disagreement that matters is the one nobody
/// notices: a player who disconnects is destroyed and takes their edge with
/// them, so this function frees their mob on the very next call and a cached
/// set would go on reserving it for somebody who left.
#[must_use]
pub fn claims(world: &World) -> Vec<Claim> {
    let mut found = Vec::new();
    world
        .query::<()>()
        .with((Playing, id::<flecs::Wildcard>()))
        .build()
        .each_entity(|player, ()| {
            let Some(kit) = player.find_target(Playing, |_| true) else {
                return;
            };
            found.push(Claim {
                kit: kit.id(),
                player: player.id(),
            });
        });
    found
}

/// The one player playing `kit`, if anybody is.
///
/// One, and not a list, because [`Playing`] is registered `Exclusive` and the
/// selection rule is one player per mob; a second holder is a bug rather than
/// a case to handle, and `tests/selector.rs` is what says so.
#[must_use]
pub fn claimant(world: &World, kit: Entity) -> Option<Entity> {
    claims(world)
        .into_iter()
        .find(|claim| claim.kit == kit)
        .map(|claim| claim.player)
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

        // A `/kit` completion is a query over whatever carries this tag, and
        // this is the one place that says what a kit's name is. Nothing else
        // in the completion path knows a kit from a map or an ability.
        //
        // `SuggestionLabel` is registered here as well as by `HyperionCore`,
        // the same way this crate registers hyperion's `Position` and `Health`:
        // `tests/contract.rs` runs each module in a world holding only what
        // that module declares, so a component reached for and never registered
        // aborts there rather than on a server.
        world.component::<SuggestionLabel>();
        world.component::<Kit>().set(SuggestionLabel(|kit| {
            kit.try_get::<&KitName>(|name| name.0.to_owned())
        }));
        world.component::<KitStats>();
        world.component::<KitName>();
        world.component::<KitCost>();
        world.component::<KitBlurb>();
        world.component::<KitMob>();
        world.component::<KitSkin>();
        world.component::<Ultimate>();
        world
            .component::<Playing>()
            .add_trait::<flecs::Relationship>()
            .add(flecs::Exclusive);
    }
}

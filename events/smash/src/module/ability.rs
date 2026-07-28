//! Abilities are entities, and a kit grants them through a relationship.
//!
//! The alternative — an `enum Ability` with a `match` in an activation
//! function — is what makes adding a kit touch existing files. Here an ability
//! is an entity carrying its own cooldown, its own hotbar binding and its own
//! behaviour as a function pointer, so the dispatcher below never learns the
//! name of a single kit.
//!
//! Behaviour is a bare `fn`, not a `Box<dyn Fn>`: activation is rare enough
//! that one indirect call costs nothing, and a boxed closure would put an
//! allocation and a second pointer chase into a path that a kit author will be
//! tempted to call from a per-tick system.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    flecs_ext::{EntityViewExt, WorldRefExt},
    module::{
        player::{Energy, Facing, Health, OnGround, Player, Position},
        sound::{self, PlaysOnCast},
        visuals,
    },
    server::{PlayerId, Server, ServerHandle},
};

/// Tag on ability prefabs and ability instances.
#[derive(Component, Debug)]
pub struct Ability;

/// Relationship: `(Grants, ability)` on a player or on a kit prefab.
///
/// A relationship rather than a `Vec<Entity>` field because grants come and go:
/// the Smash Crystal grants an ultimate for fifteen seconds and takes it back,
/// and flecs removing the edge when the ability entity dies is one less
/// invalidation rule to get wrong.
#[derive(Component, Debug)]
pub struct Grants;

/// Which hotbar slot activates this ability.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Slot(pub u8);

/// The vanilla item shown in that slot. Mineplex bound abilities to specific
/// tools — Iron Axe, Iron Shovel, Iron Pickaxe — and players learned kits by
/// their loadout, so the item is part of the ability, not decoration.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Item(pub &'static str);

/// Human-readable name, used in the hotbar tooltip and in ability messages.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Named(pub &'static str);

/// One line of tooltip.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Description(pub &'static str);

/// One thing a client can see when an ability fires.
///
/// This is the half of an ability declaration that the gates read. A kit says
/// what its ability does in terms a player could point at, and both
/// `tests/abilities.rs` and the scripted-client gate `nix run .#smash-e2e`
/// enumerate [`manifest`] and hold every ability to what it declared. An
/// ability that declares nothing fails the first test in `tests/abilities.rs`;
/// an ability that declares something it does not do fails both gates.
///
/// The list is deliberately short and stated in wire terms, because an
/// observation nothing on the far side of the seam can see is not a proof. Each
/// variant names the packet that carries it:
///
/// - [`Self::HurtsTarget`] and [`Self::HealsCaster`] are `ClientboundSetHealth`
/// - [`Self::LaunchesTarget`] and [`Self::LaunchesCaster`] are
///   `ClientboundSetEntityMotion`
/// - [`Self::TeleportsCaster`] is `ClientboundPlayerPosition`
/// - [`Self::BuffsMelee`] is the same melee swing hurting more than it did
/// - [`Self::AfflictsTarget`] is a *second* `ClientboundSetHealth` for the
///   victim, on a later tick, with nothing cast in between
/// - [`Self::Sustains`] is any of those arriving again, later, with nothing
///   pressed in between
/// - [`Self::ShieldsCaster`] is the absence of one: a hit lands on the caster
///   during the window and no `ClientboundSetHealth` follows it, and the same
///   hit after the window does
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Observable {
    /// A player in front of the caster loses health.
    HurtsTarget,
    /// A player in front of the caster gains velocity.
    LaunchesTarget,
    /// The caster gains velocity.
    LaunchesCaster,
    /// The caster ends up somewhere they were not.
    TeleportsCaster,
    /// The caster's own health bar goes up.
    HealsCaster,
    /// The caster's melee swing hurts more than it did before.
    BuffsMelee,
    /// A player the ability touched keeps losing health after it is over.
    ///
    /// The distinguishing word is *keeps*. Every ability in the game can take
    /// health off somebody once; what this claims is that the cast left
    /// something behind on the victim which is still acting on its own a
    /// second later, which is [`crate::module::effect`] and nothing else. An
    /// ability that declares it and merely hits hard fails the gate, because
    /// the gate stops casting and watches.
    AfflictsTarget,
    /// The ability goes on acting after the cast, because it left a mode on the
    /// caster rather than doing one thing.
    ///
    /// What a Smash Crystal grants. The wiki gives every ultimate as a
    /// duration -- "lasts 20 seconds", "unlimited flight and eggs", "call
    /// lightning down once a second" -- and fourteen of the fifteen were a
    /// single frame. Distinct from [`Self::AfflictsTarget`], which is something
    /// left on the *victim*: this is something left on the caster, and it is
    /// proved by the world going on changing -- more damage, more motion --
    /// through a window in which nothing at all is pressed.
    Sustains,
    /// The caster cannot be hurt for a window.
    ///
    /// Proved by the *same hit* landing before the cast and being refused after
    /// it. Both halves are needed, because "took no damage" on its own is also
    /// what a broken damage pipeline looks like, and a pair taken either side of
    /// one press rules that out without anyone having to know how long the
    /// window is.
    ///
    /// That last part matters: the two shields in the roster are one second and
    /// nineteen, and a proof shaped as "wait it out and hit again" works for the
    /// first and cannot afford the second. What the gates therefore do *not*
    /// check per ability is that the window ever ends. That is the effect
    /// module's behaviour rather than any one kit's, and `tests/contract.rs`
    /// proves it once, there.
    ShieldsCaster,
}

impl Observable {
    /// The name this observation goes over the wire under, in `/abilities`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HurtsTarget => "hurts_target",
            Self::LaunchesTarget => "launches_target",
            Self::LaunchesCaster => "launches_caster",
            Self::TeleportsCaster => "teleports_caster",
            Self::HealsCaster => "heals_caster",
            Self::BuffsMelee => "buffs_melee",
            Self::AfflictsTarget => "afflicts_target",
            Self::Sustains => "sustains",
            Self::ShieldsCaster => "shields_caster",
        }
    }
}

/// What an ability promises a client will see. Declared by the kit.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Proves(pub &'static [Observable]);

/// Seconds left on a grant that expires on its own.
///
/// The Smash Crystal is the only thing that makes one: an ultimate is granted
/// for a fixed time and taken back, and the ability entity carrying the
/// countdown is the whole of that bookkeeping.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct GrantedFor {
    pub remaining: f32,
}

/// How long after use before the ability is available again, in seconds.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct CooldownSpec(pub f32);

/// Live cooldown on a player's own instance of an ability.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Cooldown {
    pub remaining: f32,
}

/// Energy consumed per activation, for the kits that have an energy bar.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct EnergyCost(pub f32);

/// Only usable with both feet on the ground. Mineplex required this of Fissure
/// and Seismic Slam.
#[derive(Component, Debug)]
pub struct RequiresGround;

/// This ability's cooldown is cleared by landing a hit rather than by waiting.
///
/// A tag on the ability, not a behaviour: clearing the cooldown is the kit's own
/// `on_hit` payload. What this says is that the ability is *allowed* to come
/// back early, which is what stops the shared cooldown check in
/// `tests/abilities.rs` from reading a deliberate refund as a broken cooldown.
#[derive(Component, Debug)]
pub struct RefundsOnHit;

/// Everything an ability gets to see and touch when it fires.
pub struct Cast<'a> {
    pub world: WorldRef<'a>,
    pub caster: EntityView<'a>,
    pub ability: EntityView<'a>,
    pub server: &'a dyn Server,
    pub player: PlayerId,
    pub position: Position,
    pub facing: Facing,
    /// 0.0 for a tap, rising to 1.0 for a fully charged hold. Abilities without
    /// a charge always see 1.0.
    pub charge: f32,
}

/// Fired on right-click.
#[derive(Component, Copy, Clone)]
pub struct OnActivate(pub fn(&Cast<'_>));

/// Fired when a held ability is released, with the charge fraction filled in.
/// Barrage, Block Toss and Slime Rocket are all this shape.
#[derive(Component, Copy, Clone)]
pub struct OnRelease(pub fn(&Cast<'_>));

/// Seconds of holding that count as fully charged.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct ChargeTime(pub f32);

/// Live charge state, present only while a player is holding the button.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Charging {
    pub held: f32,
}

/// The host reporting a right-click, emitted at the player.
#[derive(Component, Debug, Copy, Clone)]
pub struct UseSlot(pub u8);

/// The host reporting that a held slot was released, emitted at the player.
#[derive(Component, Debug, Copy, Clone)]
pub struct ReleaseSlot(pub u8);

/// The adapter's entry point for a right-click.
pub fn use_slot(player: EntityView<'_>, slot: u8) {
    crate::module::player::notify(player, &UseSlot(slot));
}

/// The adapter's entry point for letting go of a held slot.
pub fn release_slot(player: EntityView<'_>, slot: u8) {
    crate::module::player::notify(player, &ReleaseSlot(slot));
}

/// Why an activation did not happen. Reported to the player, and useful in
/// tests as the single place a refusal is decided.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Refusal {
    OnCooldown,
    NotEnoughEnergy,
    NotGrounded,
}

impl Refusal {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::OnCooldown => "That ability is recharging.",
            Self::NotEnoughEnergy => "Not enough energy.",
            Self::NotGrounded => "You must be on the ground.",
        }
    }
}

/// Find the ability a player has bound to `slot`.
#[must_use]
pub fn granted_in_slot(player: EntityView<'_>, slot: u8) -> Option<EntityView<'_>> {
    player.find_target(Grants, |ability| {
        ability.try_get::<&Slot>(|s| s.0 == slot) == Some(true)
    })
}

/// Whether an ability is usable right now, without using it.
fn check(ability: EntityView<'_>, player: EntityView<'_>) -> Result<(), Refusal> {
    if ability.try_get::<&Cooldown>(|c| c.remaining > 0.0) == Some(true) {
        return Err(Refusal::OnCooldown);
    }
    if ability.has(RequiresGround::id()) && player.try_get::<&OnGround>(|g| g.0) != Some(true) {
        return Err(Refusal::NotGrounded);
    }
    if let Some(cost) = ability.try_get::<&EnergyCost>(|c| c.0)
        && player.try_get::<&Energy>(|e| e.current + f32::EPSILON >= cost) != Some(true)
    {
        return Err(Refusal::NotEnoughEnergy);
    }
    Ok(())
}

/// Spend the cooldown and the energy an activation costs.
fn commit(ability: EntityView<'_>, player: EntityView<'_>) {
    if let Some(spec) = ability.try_get::<&CooldownSpec>(|c| c.0) {
        ability.set(Cooldown { remaining: spec });
    }
    if let Some(cost) = ability.try_get::<&EnergyCost>(|c| c.0) {
        player.get::<&mut Energy>(|energy| {
            energy.current = (energy.current - cost).max(0.0);
        });
    }
}

/// Build a [`Cast`] for `player` firing `ability`, without firing it.
///
/// Public because an ability is not the only thing that runs an ability's
/// payload. A Smash Crystal's twenty seconds is the same payload on a beat, and
/// [`crate::module::effect::Repeats`] needs exactly this to hand one a `Cast`
/// -- so that a pulse and a press are the same function with the same helpers,
/// rather than two shapes a kit has to write its ability twice for.
///
/// `None` when the player is missing anything a cast needs, which is a player
/// who is mid-teardown.
#[must_use]
pub fn cast_from<'w>(
    world: WorldRef<'w>,
    player: EntityView<'w>,
    ability: EntityView<'w>,
    server: &'w dyn Server,
    charge: f32,
) -> Option<Cast<'w>> {
    Some(Cast {
        world,
        caster: player,
        ability,
        server,
        player: player.try_get::<&PlayerId>(|p| *p)?,
        position: player.try_get::<&Position>(|p| *p)?,
        facing: player.try_get::<&Facing>(|f| *f)?,
        charge,
    })
}

/// Run one activation. Split out from the observers so the lobby, a command and
/// a test can all drive an ability through the same gate.
pub fn activate(player: EntityView<'_>, slot: u8, charge: f32) -> Result<(), Refusal> {
    let Some(ability) = granted_in_slot(player, slot) else {
        return Ok(());
    };
    check(ability, player)?;

    // Before the payload, not after. Mineplex's `RESPAWN_INVUL` ends the moment
    // you act, which is what this is; running it afterwards also deleted a
    // window the payload had just granted, so Sky Squid's one untouchable
    // second lasted exactly no frames and nothing anywhere said so.
    player.remove(crate::module::lives::InvulnerableUntil::id());

    let world = player.world();
    world.get::<&ServerHandle>(|server| {
        let Some(cast) = cast_from(world, player, ability, &**server, charge) else {
            return;
        };
        // A charge ability's payload lives on `OnRelease`; a tap ability's on
        // `OnActivate`. Having both is not meaningful, so `OnRelease` wins.
        if let Some(f) = ability.try_get::<&OnRelease>(|f| *f) {
            f.0(&cast);
        } else if let Some(f) = ability.try_get::<&OnActivate>(|f| *f) {
            f.0(&cast);
        }
        // One place, for every ability in the game. The dispatcher still names
        // no kit: what to play is read off the ability entity that just fired.
        if let Some(sound) = sound::declared(ability, PlaysOnCast) {
            server.play_sound(cast.position.0, sound);
        }
    });

    commit(ability, player);
    Ok(())
}

/// Seconds a Smash Crystal's ultimate lasts before it is taken back.
///
/// Mineplex's crystal spawned in the arena, was picked up, and gave the holder
/// their kit's ultimate for a fixed window. The window is the ability layer's
/// business and lives here; spawning the crystal is the arena's, and until it
/// does, [`crate::module::kit::grant_ultimate`] is how one is handed out.
pub const ULTIMATE_SECONDS: f32 = 20.0;

/// One ability, exactly as its kit declared it.
///
/// The registry is the ability entities themselves; this is a flat read of them
/// for anything that wants the whole roster at once. `/abilities` serialises it
/// for the end to end gate, and `tests/abilities.rs` walks it to drive every
/// ability in the game through the mock seam.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Declared {
    pub kit: &'static str,
    pub name: &'static str,
    pub slot: u8,
    pub item: &'static str,
    pub description: &'static str,
    pub cooldown: f32,
    /// `Some` for a hold-and-release ability, in seconds to full charge.
    pub charge_time: Option<f32>,
    pub energy_cost: Option<f32>,
    pub requires_ground: bool,
    /// A hit clears the cooldown, so a second use may legitimately be allowed
    /// straight away.
    pub refunds_on_hit: bool,
    /// Granted by the Smash Crystal rather than at spawn.
    pub ultimate: bool,
    pub proves: &'static [Observable],
    /// The vanilla sound event firing it plays, read back off the ability's
    /// `(PlaysOnCast, sound)` edge. Empty for an ability that declared none,
    /// which is what `tests/sound.rs` fails on.
    pub sound: &'static str,
}

/// Every ability every registered kit declares, kit registration order first
/// and hotbar slot order within a kit.
///
/// A query over the world, not a list anybody maintains: a kit imported from
/// outside the crate appears here the moment its module runs, which is what
/// makes this the single source of truth the gates enumerate.
#[must_use]
pub fn manifest(world: &World) -> Vec<Declared> {
    use crate::module::kit::{KitName, Ultimate, registry};

    let mut out = Vec::new();
    for kit in registry(world) {
        let kit = world.entity_from_id(kit);
        let Some(kit_name) = kit.try_get::<&KitName>(|name| name.0) else {
            continue;
        };
        let mut abilities = Vec::new();
        kit.each_target_view(Grants, |ability| {
            let (Some(name), Some(slot)) = (
                ability.try_get::<&Named>(|n| n.0),
                ability.try_get::<&Slot>(|s| s.0),
            ) else {
                return;
            };
            abilities.push(Declared {
                kit: kit_name,
                name,
                slot,
                item: ability.try_get::<&Item>(|i| i.0).unwrap_or(""),
                description: ability.try_get::<&Description>(|d| d.0).unwrap_or(""),
                cooldown: ability.try_get::<&CooldownSpec>(|c| c.0).unwrap_or(0.0),
                charge_time: ability.try_get::<&ChargeTime>(|c| c.0),
                energy_cost: ability.try_get::<&EnergyCost>(|c| c.0),
                requires_ground: ability.has(RequiresGround::id()),
                refunds_on_hit: ability.has(RefundsOnHit::id()),
                ultimate: ability.has(Ultimate::id()),
                proves: ability.try_get::<&Proves>(|p| p.0).unwrap_or(&[]),
                sound: crate::module::sound::declared(ability, PlaysOnCast)
                    .map_or("", |sound| sound.id),
            });
        });
        abilities.sort_by_key(|ability| ability.slot);
        out.append(&mut abilities);
    }
    out
}

/// Take back a grant that has run out: unlink it from whoever holds it and
/// destroy the instance.
fn expire(world: WorldRef<'_>, expired: &[Entity]) {
    let mut edges = Vec::new();
    world
        .query::<()>()
        .with(Player::id())
        .build()
        .each_entity(|player, ()| {
            player.each_target_view(Grants, |ability| {
                if expired.contains(&ability.id()) {
                    edges.push((player.id(), ability.id()));
                }
            });
        });
    for (player, ability) in edges {
        world.entity_at(player).remove((Grants, ability));
    }
    for ability in expired {
        let ability = world.entity_at(*ability);
        if ability.is_alive() {
            ability.destruct();
        }
    }
}

#[derive(Component)]
pub struct AbilityModule;

impl Module for AbilityModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Ability");

        world.component::<Ability>();
        world.component::<Slot>();
        world.component::<Item>();
        world.component::<Named>();
        world.component::<Description>();
        world.component::<CooldownSpec>();
        world.component::<Cooldown>();
        world.component::<EnergyCost>();
        world.component::<RequiresGround>();
        world.component::<RefundsOnHit>();
        world.component::<OnActivate>();
        world.component::<OnRelease>();
        world.component::<ChargeTime>();
        world.component::<Charging>();
        world.component::<UseSlot>();
        world.component::<ReleaseSlot>();
        world.component::<Grants>();
        world.component::<Proves>();
        world.component::<GrantedFor>();

        world
            .system_named::<&mut Cooldown>("smash::tick_cooldowns")
            .each_iter(|it, _, cooldown| {
                if cooldown.remaining > 0.0 {
                    cooldown.remaining = (cooldown.remaining - it.delta_time()).max(0.0);
                }
            });

        world
            .system_named::<&mut Charging>("smash::tick_charge")
            .each_iter(|it, _, charging| {
                charging.held += it.delta_time();
            });

        // Grants that expire. Written as one `run` rather than as a per-entity
        // system because taking the grant back edits the holder's type, and
        // flecs refuses that from inside the query that found it.
        world
            .system_named::<()>("smash::expire_grants")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    let dt = it.delta_time();
                    let mut expired = Vec::new();
                    world
                        .query::<&mut GrantedFor>()
                        .build()
                        .each_entity(|ability, granted| {
                            granted.remaining -= dt;
                            if granted.remaining <= 0.0 {
                                expired.push(ability.id());
                            }
                        });
                    if !expired.is_empty() {
                        expire(world, &expired);
                    }
                }
            });

        // A single dispatcher for every ability in the game. Adding a kit
        // cannot require editing it, because it never names one.
        world
            // `Player` is a tag, so it is named as a filter term rather than a
            // data term: asking for `&Player` fails a const assertion deep in
            // flecs with no mention of the tag. See the API notes.
            .observer_named::<UseSlot, ()>("smash::on_use_slot")
            .with(Player::id())
            .each_iter(|it, index, ()| {
                let slot = it.param().0;
                let player = it.entity(index);

                // A held ability starts charging instead of firing.
                if let Some(ability) = granted_in_slot(player, slot)
                    && ability.has(ChargeTime::id())
                {
                    // Checked here and not only on release: a charge that is
                    // going to be refused should be refused while the player
                    // can still do something else, and a hold that silently
                    // banked a use is how a cooldown looks like it does not
                    // exist from a client's point of view.
                    if let Err(refusal) = check(ability, player) {
                        report(player, Err(refusal));
                        return;
                    }
                    ability.set(Charging { held: 0.0 });
                    return;
                }

                report(player, activate(player, slot, 1.0));
            });

        world
            .observer_named::<ReleaseSlot, ()>("smash::on_release_slot")
            .with(Player::id())
            .each_iter(|it, index, ()| {
                let slot = it.param().0;
                let player = it.entity(index);

                let Some(ability) = granted_in_slot(player, slot) else {
                    return;
                };
                let held = ability.try_get::<&Charging>(|c| c.held).unwrap_or(0.0);
                let full = ability.try_get::<&ChargeTime>(|c| c.0).unwrap_or(1.0);
                ability.remove(Charging::id());

                report(
                    player,
                    activate(player, slot, (held / full).clamp(0.0, 1.0)),
                );
            });
    }
}

fn report(player: EntityView<'_>, outcome: Result<(), Refusal>) {
    let Err(refusal) = outcome else {
        return;
    };
    let Some(id) = player.try_get::<&PlayerId>(|p| *p) else {
        return;
    };
    player.world().get::<&ServerHandle>(|server| {
        // Red because a refusal is a refusal, and the action bar is the only
        // place a player sees it.
        server.send_message(
            id,
            crate::server::Channel::ActionBar,
            crate::server::Text::text(refusal.message()).color(crate::server::NamedColor::Red),
        );
    });
}

/// Hurt everything within `radius` of `at`, except the caster, launching it away
/// from `origin`.
///
/// The one geometric primitive the kits share. Collecting the victims before
/// hurting any of them is deliberate: the damage observers mutate components
/// the query is reading, and flecs will catch that at runtime if you nest them.
///
/// `origin` is separate from `at` because knockback is horizontal-only and a
/// launch away from a point a victim is standing on has no direction: it
/// normalises to zero and the victim does not move. An ability centred on its
/// own victims -- Storm Squid calls down a bolt on each player where they stand
/// -- has to name the caster as the origin or it silently deals damage and no
/// knockback at all.
///
/// Returns whoever it hit, so an ability whose blast leaves something behind --
/// a burn, a poison, a mark -- can reach those victims without running the same
/// query a second time and getting a different answer because the first pass
/// moved somebody.
///
/// Deliberately not `#[must_use]`: most of the roster fires a splash for what it
/// does and not for who it hit, and forcing thirty call sites to discard a list
/// they never wanted would bury the handful that do use it.
#[expect(
    clippy::must_use_candidate,
    reason = "most callers want the blast, not the list; see above"
)]
pub fn splash_from(
    cast: &Cast<'_>,
    origin: Vec3,
    at: Vec3,
    radius: f32,
    damage: f32,
    multiplier: f32,
) -> Vec<Entity> {
    use crate::module::{
        damage::{DamageKind, Damaged},
        knockback::Knockback,
    };

    let caster = cast.caster.id();
    let mut victims = Vec::new();
    cast.world
        .query::<(&Position, &Health)>()
        .with(Player::id())
        .build()
        .each_entity(|entity, (position, health)| {
            if entity.id() != caster && !health.is_dead() && position.0.distance(at) <= radius {
                victims.push(entity.id());
            }
        });

    for victim in &victims {
        crate::module::damage::hurt(cast.world.entity_at(*victim), Damaged {
            attacker: Some(caster),
            amount: damage,
            knockback: Knockback::from(origin).times(multiplier),
            kind: DamageKind::Ability,
        });
    }
    victims
}

/// [`splash_from`] with the blast's own centre as the origin, which is what an
/// explosion somewhere other than on top of a victim wants.
#[expect(
    clippy::must_use_candidate,
    reason = "see splash_from: the victims are the rare want, not the usual one"
)]
pub fn splash_at(
    cast: &Cast<'_>,
    at: Vec3,
    radius: f32,
    damage: f32,
    multiplier: f32,
) -> Vec<Entity> {
    splash_from(cast, at, at, radius, damage, multiplier)
}

/// Turn a 0..=1 charge fraction into a whole number of steps.
///
/// Barrage's arrow count and Slime Rocket's size are both this shape.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 0..=max before the cast"
)]
pub fn charge_steps(charge: f32, max: u32) -> u32 {
    (charge.clamp(0.0, 1.0) * f32::from(u16::try_from(max).unwrap_or(u16::MAX))).round() as u32
}

/// [`splash_at`] centred on the caster, with the bang.
#[expect(
    clippy::must_use_candidate,
    reason = "see splash_from: the victims are the rare want, not the usual one"
)]
pub fn splash(cast: &Cast<'_>, radius: f32, damage: f32, multiplier: f32) -> Vec<Entity> {
    let victims = splash_at(cast, cast.position.0, radius, damage, multiplier);
    cast.server.particles(visuals::blast(cast.position.0));
    victims
}

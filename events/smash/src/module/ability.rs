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
    module::player::{Energy, Facing, Health, OnGround, Player, Position},
    server::{Cue, PlayerId, Server, ServerHandle},
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

fn cast_from<'w>(
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
    });

    commit(ability, player);
    // Mineplex's `RESPAWN_INVUL` ends the moment you act, so a player cannot
    // spend it attacking from under a platform nobody can answer from.
    player.remove(crate::module::lives::InvulnerableUntil::id());
    Ok(())
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
        world.component::<OnActivate>();
        world.component::<OnRelease>();
        world.component::<ChargeTime>();
        world.component::<Charging>();
        world.component::<UseSlot>();
        world.component::<ReleaseSlot>();
        world.component::<Grants>();

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
        server.send_message(id, crate::server::Channel::ActionBar, refusal.message());
    });
}

/// Hurt everything within `radius` of `at`, except the caster.
///
/// The one geometric primitive the kits share. Collecting the victims before
/// hurting any of them is deliberate: the damage observers mutate components
/// the query is reading, and flecs will catch that at runtime if you nest them.
pub fn splash_at(cast: &Cast<'_>, at: Vec3, radius: f32, damage: f32, multiplier: f32) {
    use crate::module::{
        damage::{DamageKind, Damaged},
        knockback::Knockback,
    };

    let caster = cast.caster.id();
    let mut victims = Vec::new();
    cast.world
        .query::<(&Position, &Health)>()
        .build()
        .each_entity(|entity, (position, health)| {
            if entity.id() != caster && !health.is_dead() && position.0.distance(at) <= radius {
                victims.push(entity.id());
            }
        });

    for victim in victims {
        crate::module::damage::hurt(cast.world.entity_at(victim), Damaged {
            attacker: Some(caster),
            amount: damage,
            knockback: Knockback::from(at).times(multiplier),
            kind: DamageKind::Ability,
        });
    }
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
pub fn splash(cast: &Cast<'_>, radius: f32, damage: f32, multiplier: f32) {
    splash_at(cast, cast.position.0, radius, damage, multiplier);
    cast.server.cue(cast.position.0, Cue::Explosion);
}

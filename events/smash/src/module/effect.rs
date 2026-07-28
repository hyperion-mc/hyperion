//! An effect is an entity, and afflicting somebody is an edge pointing at them.
//!
//! Fire, poison and a shield have nothing in common as fiction and everything
//! in common as bookkeeping: each is a thing put on a player at one moment,
//! which keeps acting on its own for a while, and then stops. Writing that three
//! times produces three expiry bugs, so it is written once here and the kits
//! differ only in the numbers they hand to [`afflict`].
//!
//! The alternative -- an `Option<f32>` per effect on the player, or an
//! `enum Status` with a match on it -- is the shape this crate spends its design
//! avoiding, and it also cannot express the thing that actually happens in a
//! match, which is being on fire from two different Blazes at once. Two effect
//! entities pointing at one victim is that, for free, and each keeps its own
//! kill credit because the attacker is an edge on the effect rather than a
//! field on the player.
//!
//! **Where this sits in the module DAG.** It needs `Player`, `Knockback` and
//! `Damage`, and nothing else, to import: a tick is a [`Damaged`] event and a
//! shield is `Damage`'s own [`Immune`] tag. The edge points one way -- `Damage`
//! knows nothing of effects -- which is why the tag lives over there rather
//! than here. It deliberately does *not* need `Lobby`: durations
//! are counted in delta time the way [`crate::module::ability::Cooldown`] and
//! `GrantedFor` are, not against the match clock, so an effect behaves the same
//! in the hub, in a test and in a match. `tests/contract.rs` states all of that
//! and holds it.
//!
//! What does not live here is anything a vanilla client renders for itself.
//! Minecraft has real Slowness and real Speed, delivered by
//! `ClientboundUpdateMobEffect`, and approximating either with repeated
//! impulses would read as lag rather than as a status. Those wait for the seam
//! to carry the packet.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    flecs_ext::WorldRefExt,
    module::{
        ability::Cast,
        damage::{DamageKind, Damaged, Immune, hurt},
        knockback::Knockback,
        player::{Health, Position},
        projectile::Impact,
    },
    server::{Cue, ServerHandle, Sound, SoundCategory},
};

/// Tag on effect entities.
#[derive(Component, Debug)]
pub struct Effect;

/// Relationship: `(Afflicting, victim)` on the effect entity.
///
/// Pointing from the effect at the player rather than the other way around,
/// because one player carries many effects and a relationship is cheapest to
/// read in the direction that has one target. It also means a player who
/// disconnects takes their effects' edges with them, so an expiry firing
/// afterwards finds no target instead of hurting whoever recycled the id.
#[derive(Component, Debug)]
pub struct Afflicting;

/// Relationship: `(InflictedBy, attacker)` on the effect entity.
///
/// Kill credit for a burn belongs to whoever lit it, and has to survive that
/// player changing kit or dying first, which is what makes it an edge rather
/// than an `Entity` field.
#[derive(Component, Debug)]
pub struct InflictedBy;

/// Relationship: `(Source, thing)` on the effect entity.
///
/// What makes "this effect again" a question with an answer. See [`Blame`].
#[derive(Component, Debug)]
pub struct Source;

/// Seconds the effect has left.
///
/// Counted down by delta time rather than compared against
/// [`crate::module::damage::MatchClock`], which advances only in
/// `Phase::Playing`. An effect that stopped ticking the moment a match ended
/// would be a burn that pauses on the results screen, and -- the reason this
/// was found -- one that never ticks at all in the hub or in a test.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Expires {
    pub remaining: f32,
}

/// Health this effect takes, and how often it takes it.
///
/// Fire and poison differ only in these numbers and in the [`DamageKind`] that
/// decides whether armour applies, so they are one component and one system
/// rather than two of each. Blaze exists because its damage ignores armour, so
/// the kind travelling per effect rather than being fixed here is the point.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Ticks {
    /// Health per application, before armour.
    pub amount: f32,
    pub kind: DamageKind,
    /// Seconds between applications.
    pub interval: f32,
    /// Seconds until the next one.
    pub until_next: f32,
}

/// What a client sees and hears on each application.
///
/// Carried on the effect rather than decided by the system, because a burn and
/// a poison are the same mechanic and this is the only thing that separates
/// them for a player. Carried as one value so an effect cannot be given a noise
/// and no picture.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Shows {
    pub cue: Cue,
    /// A vanilla sound event id, held against the generated registry by
    /// `tests/sound.rs` like every other sound in the game.
    pub sound: &'static str,
}

/// While this stands, the victim cannot be hurt.
///
/// A tag and not a duration: the duration is [`Expires`], and a shield that
/// protected for a different window than it lasted would be two numbers with a
/// standing invitation to disagree.
#[derive(Component, Debug)]
pub struct Shields;

/// Everything one call puts on a victim.
///
/// Filled in with update syntax like [`crate::module::kit::AbilitySpec`], so a
/// kit writes the two fields its effect has instead of a wall of `None`.
#[derive(Debug, Copy, Clone)]
pub struct Affliction {
    /// How long it stands, in seconds.
    pub seconds: f32,
    /// `Some` for damage over time.
    pub ticks: Option<Ticks>,
    /// `Some` for something a client can see happening.
    pub shows: Option<Shows>,
    /// Whether the victim is untouchable while it stands.
    pub shields: bool,
}

impl Affliction {
    pub const DEFAULT: Self = Self {
        seconds: 0.0,
        ticks: None,
        shows: None,
        shields: false,
    };

    /// Damage over time that a client can see, which is the only kind any kit
    /// has wanted: an invisible burn is indistinguishable from the server
    /// deciding to hurt you for no reason.
    #[must_use]
    pub const fn over_time(
        seconds: f32,
        amount: f32,
        interval: f32,
        kind: DamageKind,
        shows: Shows,
    ) -> Self {
        Self {
            seconds,
            ticks: Some(Ticks {
                amount,
                kind,
                interval,
                // One interval after the cast, so the first tick is a separate
                // event a player can feel rather than arriving inside the hit
                // that caused it and reading as one larger hit.
                until_next: interval,
            }),
            shows: Some(shows),
            shields: false,
        }
    }

    /// A window in which nothing can touch the holder.
    #[must_use]
    pub const fn shield(seconds: f32) -> Self {
        Self {
            seconds,
            shields: true,
            ..Self::DEFAULT
        }
    }
}

impl Default for Affliction {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What an effect counts as "the same as", and who to credit for it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Blame {
    /// The entity identifying this effect. Two afflictions with the same source
    /// on one victim are the same affliction re-applied, and the second
    /// replaces the first.
    ///
    /// A cast names its own ability instance, so one Blaze refreshing their
    /// Inferno burn and two Blazes each landing one are told apart. A
    /// projectile impact names the shooter instead, because a projectile does
    /// not yet carry which ability fired it -- the same gap that makes Chicken
    /// Missile look its own ability up by name string. Both readings mean "this
    /// again, from the same place"; when a projectile carries its ability the
    /// impact path narrows to match the cast path and nothing here changes.
    pub source: Entity,
    /// Who gets the kill if this is what finishes the victim.
    pub attacker: Entity,
}

impl Blame {
    /// From an ability firing.
    #[must_use]
    pub fn cast(cast: &Cast<'_>) -> Self {
        Self {
            source: cast.ability.id(),
            attacker: cast.caster.id(),
        }
    }

    /// From a projectile connecting. `None` when nobody owns the projectile,
    /// which the flight system allows and no kit currently produces.
    #[must_use]
    pub fn impact(impact: &Impact<'_>) -> Option<Self> {
        let shooter = impact.shooter?.id();
        Some(Self {
            source: shooter,
            attacker: shooter,
        })
    }
}

/// Put `affliction` on `victim`.
///
/// Replaces rather than stacks: anything already on `victim` from the same
/// [`Blame::source`] is destroyed first, so re-applying refreshes the clock.
/// Blaze's Inferno has a half-second cooldown, and without this a Blaze holding
/// the trigger would pile up forty overlapping burns on one victim.
///
/// Hands nothing back, for the reason [`crate::module::projectile::fire`] does
/// not: the effect entity's whole life is owned by the system below, and a
/// caller holding one past the tick it expires on is the only way to get a
/// dangling id out of this module.
pub fn afflict(world: WorldRef<'_>, victim: EntityView<'_>, blame: Blame, affliction: Affliction) {
    for existing in from_source(world, victim.id(), blame.source) {
        let existing = world.entity_at(existing);
        if existing.is_alive() {
            existing.destruct();
        }
    }

    let effect = world
        .new_entity()
        .add(Effect::id())
        .set(Expires {
            remaining: affliction.seconds,
        })
        .add((Afflicting, victim))
        .add((Source, world.entity_at(blame.source)))
        .add((InflictedBy, world.entity_at(blame.attacker)));

    if let Some(ticks) = affliction.ticks {
        effect.set(ticks);
    }
    if let Some(shows) = affliction.shows {
        effect.set(shows);
    }
    if affliction.shields {
        effect.add(Shields::id());
        arm_shield(victim);
    }
}

/// Turn the damage pipeline's immunity tag on for `victim`.
///
/// A tag and not a deadline. What ends an effect's shield is the effect
/// expiring, which [`disarm_shield`] is, so a second copy of the duration on the
/// player would be a number with nothing to keep it honest.
///
/// The pipeline reading a tag rather than querying for effects is deliberate:
/// `smash::apply_damage` runs on every hit in the game, and a relationship walk
/// per hit to answer a question that is false for almost every player almost
/// always is the wrong shape. [`crate::module::damage::Immune`] is where that
/// tag lives and why it is not the kill plane's flag.
fn arm_shield(victim: EntityView<'_>) {
    victim.add(Immune::id());
}

/// Turn it off again, unless something else is still shielding them.
fn disarm_shield(world: WorldRef<'_>, victim: Entity, excluding: Entity) {
    let others = matching(world, victim, |effect| {
        effect.id() != excluding && effect.has(Shields::id())
    });
    if others.is_empty() {
        world.entity_at(victim).remove(Immune::id());
    }
}

/// Every effect standing on `victim` that `keep` accepts.
///
/// One walker rather than a query per question: every one of those questions is
/// "which effects point at this player" with a different filter, and three
/// copies of a relationship traversal is three places to get the direction of
/// the edge wrong.
fn matching(
    world: WorldRef<'_>,
    victim: Entity,
    keep: impl Fn(EntityView<'_>) -> bool,
) -> Vec<Entity> {
    let mut found = Vec::new();
    world
        .query::<()>()
        .with(Effect::id())
        .build()
        .each_entity(|effect, ()| {
            if effect.target(Afflicting, 0).map(|target| target.id()) == Some(victim)
                && keep(effect)
            {
                found.push(effect.id());
            }
        });
    found
}

/// Effects on `victim` that came from `source`. See [`Blame::source`].
#[must_use]
pub fn from_source(world: WorldRef<'_>, victim: Entity, source: Entity) -> Vec<Entity> {
    matching(world, victim, |effect| {
        effect.target(Source, 0).map(|from| from.id()) == Some(source)
    })
}

/// Everything currently afflicting `victim`.
#[must_use]
pub fn on(world: WorldRef<'_>, victim: Entity) -> Vec<Entity> {
    matching(world, victim, |_| true)
}

/// Take every effect off `victim`.
///
/// Called on respawn: arriving back on the map still burning from the life you
/// already lost is not something anybody reads as a feature.
pub fn clear(world: WorldRef<'_>, victim: Entity) {
    for effect in on(world, victim) {
        let effect = world.entity_at(effect);
        if effect.is_alive() {
            end(world, effect);
        }
    }
}

/// Destroy one effect, undoing whatever it turned on.
fn end(world: WorldRef<'_>, effect: EntityView<'_>) {
    if effect.has(Shields::id())
        && let Some(victim) = effect.target(Afflicting, 0)
    {
        disarm_shield(world, victim.id(), effect.id());
    }
    effect.destruct();
}

/// One application of one effect, decided before anything is mutated.
struct Application {
    victim: Entity,
    attacker: Option<Entity>,
    amount: f32,
    kind: DamageKind,
    shows: Option<Shows>,
}

#[derive(Component)]
pub struct EffectModule;

impl Module for EffectModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Effect");

        world.component::<Effect>();
        world.component::<Expires>();
        world.component::<Ticks>();
        world.component::<Shows>();
        world.component::<Shields>();
        // Exclusive: an effect afflicts exactly one player, comes from one
        // place and is owed to one attacker. A second target would silently
        // double every tick it deals.
        world.component::<Afflicting>().add(flecs::Exclusive);
        world.component::<InflictedBy>().add(flecs::Exclusive);
        world.component::<Source>().add(flecs::Exclusive);

        // Applying and expiring are one `run` rather than two per-entity
        // systems, for the reason `smash::expire_grants` is: hurting a victim
        // writes `Health`, which the damage observers read, and flecs refuses
        // that from inside the query that found the effect. Everything is
        // decided first and applied afterwards.
        world
            .system_named::<()>("smash::tick_effects")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    let dt = it.delta_time();

                    let mut applications = Vec::new();
                    let mut finished = Vec::new();

                    world
                        .query::<&mut Expires>()
                        .with(Effect::id())
                        .build()
                        .each_entity(|effect, expires| {
                            let Some(victim) = effect.target(Afflicting, 0) else {
                                // The player it was put on has gone. Collected
                                // so the entity does not leak for the rest of
                                // the match.
                                finished.push(effect.id());
                                return;
                            };

                            expires.remaining -= dt;
                            if expires.remaining <= 0.0 || !victim.is_alive() {
                                finished.push(effect.id());
                                return;
                            }

                            // A dead player is not burned further: the death
                            // path owns them until they respawn, and a tick
                            // landing in that window re-kills somebody who is
                            // already spectating.
                            if victim.try_get::<&Health>(|health| health.is_dead()) != Some(false) {
                                return;
                            }

                            let Some(mut ticks) = effect.try_get::<&Ticks>(|ticks| *ticks) else {
                                return;
                            };
                            ticks.until_next -= dt;
                            if ticks.until_next > 0.0 {
                                effect.set(ticks);
                                return;
                            }
                            // Reset by adding an interval rather than by
                            // assigning one, so a long frame does not push every
                            // later tick out by however far this one overshot.
                            ticks.until_next += ticks.interval;
                            effect.set(ticks);

                            applications.push(Application {
                                victim: victim.id(),
                                attacker: effect.target(InflictedBy, 0).map(|by| by.id()),
                                amount: ticks.amount,
                                kind: ticks.kind,
                                shows: effect.try_get::<&Shows>(|shows| *shows),
                            });
                        });

                    for application in applications {
                        let victim = world.entity_at(application.victim);
                        let at = victim.try_get::<&Position>(|position| position.0);

                        hurt(victim, Damaged {
                            attacker: application.attacker,
                            amount: application.amount,
                            // No knockback. Mineplex's damage over time moved
                            // nobody, and a burn nudging a player every second
                            // would fight their own movement for the whole
                            // duration and read as rubber-banding.
                            knockback: Knockback::from(Vec3::ZERO).times(0.0),
                            kind: application.kind,
                        });

                        // The picture and the damage are the same event, in one
                        // loop iteration, so there is no arrangement of this
                        // code in which a player is hurt by something invisible.
                        let (Some(at), Some(shows)) = (at, application.shows) else {
                            continue;
                        };
                        world.get::<&ServerHandle>(|server| {
                            server.cue(at, shows.cue);
                            server.play_sound(at, Sound::new(shows.sound, SoundCategory::Players));
                        });
                    }

                    for effect in finished {
                        let effect = world.entity_at(effect);
                        if effect.is_alive() {
                            end(world, effect);
                        }
                    }
                }
            });
    }
}

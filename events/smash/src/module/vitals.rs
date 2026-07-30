//! The two clocks that run on a player for as long as a match does.
//!
//! Health regeneration and hunger are the only things in Super Smash Mobs that
//! change a player's condition without anybody doing anything, and they pull in
//! opposite directions. Regen slowly undoes the knockback percentage a player
//! has accumulated, so a fight you walk away from is a fight you recover from.
//! Hunger makes walking away cost something, and is what Mineplex has instead
//! of sudden death: `[SOURCE]` + `[changelog]` there is no timer that ends a
//! stalled match, only a food bar that empties and then kills everyone still
//! standing around.
//!
//! Both were declared on every kit and implemented by nothing until ENG-11450.
//! `KitStats::regen` had sixteen distinct tuned values and no reader at all.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        damage::{DamageKind, Damaged, STARVE_DAMAGE, hurt},
        knockback::{Knockback, Smashed},
        lobby::{Lobby, Phase},
        player::{Health, Player, Position},
    },
    server::{PlayerId, ServerHandle},
};

/// Health points healed per second, copied off the kit's `regen`.
///
/// `[INFERRED]` The unit is health points -- half-hearts -- per second, the
/// same unit [`Health`] is counted in, which is what the kit table in
/// `docs/smash-design.md` means by its "Regen (HP/s)" column. The wiki lists
/// the stat as bare numbers ("0.35 Regen Per Second") and its Slime page
/// glosses Slime's 0.35 as "regenerating 1 heart in just four seconds". That
/// prose fits neither reading: half-hearts per second gives 5.7 s to the heart
/// and hearts per second gives 2.9 s. One loose sentence is not enough to move
/// off the unit the rest of this crate already counts health in, so it stays,
/// and the disagreement is written down here rather than being rediscovered.
///
/// A component on the player rather than a read through `(Playing, kit)` for
/// the same reason [`crate::module::knockback::KnockbackTaken`] is one: it is
/// a number a kit may later want to change mid-match.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Regen(pub f32);

/// A full food bar, in vanilla's food points: ten shanks of two points each.
pub const FULL: u8 = 20;

/// Seconds between starve ticks once the bar is empty.
///
/// `[SOURCE]` + `[changelog]` "Hunger now deals true damage (0.5 hearts per
/// tick)". Half a heart is [`STARVE_DAMAGE`], and the tick is vanilla's
/// once-a-second starvation tick, which is the clock Mineplex left alone.
pub const STARVE_INTERVAL: f32 = 1.0;

/// Food points a landed hit puts back.
///
/// `[APPROXIMATED]`. That hitting people feeds you is not a guess -- the wiki
/// is explicit that the bar "can be filled back up by hitting other mobs with
/// melee or your special skills", and the game's own front page leads with
/// "attack enemies to refill your hunger bar" -- but no source anywhere states
/// the amount. One point is half a shank, which is exactly what one drain tick
/// takes away, so the rule this ships with is *a hit buys back an interval*:
/// a player landing a hit more often than every 7.75 seconds never starves,
/// and one who stops fighting has about 155 seconds before the bar is empty.
/// Tune this and nothing else changes; it is deliberately the only number in
/// the mechanic that is ours.
pub const FOOD_PER_HIT: u8 = 1;

/// A player's food bar and the clock that drains it.
///
/// One component and not three, because a kit change resets all of it at once
/// and there is no state where two of the three are current and the third is
/// from the previous kit.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Hunger {
    /// Food points remaining, <code>0..=[FULL]</code>.
    pub food: u8,
    /// Seconds between losing one food point. The kit's `hunger_interval`.
    pub interval: f32,
    /// Seconds accumulated toward the next drain, or toward the next starve
    /// tick once [`Hunger::food`] is zero.
    pub elapsed: f32,
}

impl Hunger {
    /// A full bar draining every `interval` seconds.
    #[must_use]
    pub const fn full(interval: f32) -> Self {
        Self {
            food: FULL,
            interval,
            elapsed: 0.0,
        }
    }

    #[must_use]
    pub const fn is_starving(self) -> bool {
        self.food == 0
    }

    /// Put `points` back on the bar, up to [`FULL`]. Returns whether the
    /// number changed, so a caller knows whether the client needs telling.
    pub fn feed(&mut self, points: u8) -> bool {
        let fed = self.food.saturating_add(points).min(FULL);
        let changed = fed != self.food;
        self.food = fed;
        // Eating resets the clock as well as the number. Without this a player
        // fed on the last instant before a drain tick loses the point they just
        // earned, which reads as the refill not having worked.
        if changed {
            self.elapsed = 0.0;
        }
        changed
    }
}

impl Default for Hunger {
    /// A bar that never drains.
    ///
    /// Hunger is a kit stat, so a player with no kit has no hunger clock, and
    /// an infinite interval says that without a second "is this player playing"
    /// flag for [`drain`] to consult. [`crate::module::kit::apply`] replaces it
    /// with the kit's the moment one is chosen.
    fn default() -> Self {
        Self::full(f32::INFINITY)
    }
}

/// The number of half-hearts a vanilla client draws for `health`.
///
/// The client is sent `current * 20 / max` and renders it on a twenty-step
/// bar, so this is the finest change anybody can actually see whatever the
/// kit's pool -- which makes it the right threshold for deciding whether a
/// regeneration tick is worth a packet.
///
/// A whole number and not the float, because the caller compares two of these
/// and comparing floats for equality is both a lint and a bad habit; rounding
/// to the step the client draws is the entire point, so the integer is the
/// honest type. Clamped rather than assumed in range: `Health::heal` caps at
/// `max` today, and a helper that returns nonsense if that ever changes is a
/// worse failure than one that saturates.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=20 on the line above, which is inside u8 by a wide margin"
)]
fn drawn(health: Health) -> u8 {
    if health.max <= 0.0 {
        return 0;
    }
    let steps = (health.current * VANILLA_BAR_STEPS / health.max).ceil();
    steps.clamp(0.0, VANILLA_BAR_STEPS) as u8
}

/// Steps on a vanilla health bar: ten hearts of two.
const VANILLA_BAR_STEPS: f32 = 20.0;

/// Whether the match is running.
///
/// Both clocks are gated on it. In the hub there is nothing to heal from, and a
/// food bar that drained while a lobby waited for a second player would have
/// somebody starving on the spawn platform before the countdown started.
fn playing(world: &WorldRef<'_>) -> bool {
    world.cloned::<&Lobby>().phase == Phase::Playing
}

/// Registration only: the components, and the guarantee every player carries
/// them. No systems -- see `CLAUDE.md`.
#[derive(Component)]
pub struct VitalsComponentsModule;

impl Module for VitalsComponentsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::VitalsComponents");

        world.component::<Regen>();
        world.component::<Hunger>();
        // This module is what makes the two numbers mean anything, so this
        // module is what says every player has them.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Regen)>()
            .add_trait::<(flecs::With, Hunger)>();
    }
}

/// The two timers.
#[derive(Component)]
pub struct VitalsModule;

impl Module for VitalsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Vitals");
        world.import::<VitalsComponentsModule>();

        world.system_named::<()>("regenerate_health").run(|mut it| {
            while it.next() {
                let world = it.world();
                if !playing(&world) {
                    continue;
                }
                let dt = it.delta_time();
                // Health healed here has to be *sent*, and this is the one
                // `Health` writer in the crate that cannot rely on anything
                // else to do it. `OnSet` does not fire for a `&mut` query
                // term -- flecs reaches `flecs_notify_on_set` from `ecs_set`,
                // `ecs_modified` and add-with-value only -- so
                // `input::mirror_health_to_host` never sees this write and no
                // packet is queued. Every other writer (`damage.rs`,
                // `lives.rs`, Cow, Spider) pairs its write with an explicit
                // push; this one did not, and the result was a boss bar that
                // read the component and moved while the client's hearts,
                // which only move on a packet, did not.
                let mut told = Vec::new();
                world
                    .query::<(&Regen, &mut Health, &PlayerId)>()
                    .with(Player::id())
                    .build()
                    .each(|(regen, health, id)| {
                        // A corpse does not heal, and this is not a
                        // cosmetic rule. Zero health is the kill plane's
                        // cue -- `arena::bounds_checks` eliminates anybody
                        // `is_dead` -- and it reads that within the same
                        // tick as this runs, in whichever order the two
                        // systems happen to be registered. A heal off zero
                        // would therefore be regeneration quietly
                        // cancelling deaths, on some ticks and not others.
                        if health.is_dead() {
                            return;
                        }
                        // Before and after, both in hand on this tick, which
                        // is what lets the push be change-detected with **no
                        // cached last-sent value anywhere**. A cache would be
                        // the third instance of the class that has bitten this
                        // PR twice already -- seam state that has to be
                        // invalidated when a player dies, respawns or changes
                        // kit -- and comparing two numbers from the same tick
                        // needs no invalidating.
                        let before = *health;
                        health.heal(regen.0 * dt);
                        // Only when the client would draw something different.
                        // A vanilla health bar has twenty steps however large
                        // the kit's pool is, so anything finer is a packet per
                        // player per tick for a change nobody can see -- the
                        // same reason `hud::quantise` exists for the boss bar.
                        if drawn(before) != drawn(*health) {
                            told.push((*id, health.current, health.max));
                        }
                    });

                world.get::<&ServerHandle>(|server| {
                    for (id, current, max) in told {
                        server.set_health(id, current, max);
                    }
                });
            }
        });

        world.system_named::<()>("drain_hunger").run(|mut it| {
            while it.next() {
                let world = it.world();
                if !playing(&world) {
                    continue;
                }
                let dt = it.delta_time();

                // Decided first and applied afterwards, for the reason
                // `smash::tick_effects` is: starving hurts, and `hurt` runs the
                // whole damage pipeline, which writes the `Health` the query
                // below is holding open. flecs refuses that from inside the
                // query that found the player.
                let mut starved = Vec::new();
                let mut told = Vec::new();

                world
                    .query::<(&mut Hunger, &Health, &PlayerId)>()
                    .with(Player::id())
                    .build()
                    .each_entity(|player, (hunger, health, id)| {
                        // The death path owns a dead player until they respawn.
                        // Starving one is damage landing on somebody already
                        // spectating.
                        if health.is_dead()
                            || !(hunger.interval.is_finite() && hunger.interval > 0.0)
                        {
                            return;
                        }

                        hunger.elapsed += dt;

                        // Advanced by subtracting an interval rather than by
                        // zeroing, so a long frame does not push every later
                        // tick out by however far this one overshot. A `while`
                        // rather than an `if` for the same reason: a frame
                        // longer than the interval owes more than one tick.
                        while hunger.food > 0 && hunger.elapsed >= hunger.interval {
                            hunger.elapsed -= hunger.interval;
                            hunger.food -= 1;
                            told.push((*id, hunger.food));
                        }
                        while hunger.is_starving() && hunger.elapsed >= STARVE_INTERVAL {
                            hunger.elapsed -= STARVE_INTERVAL;
                            starved.push(player.id());
                        }
                    });

                world.get::<&ServerHandle>(|server| {
                    for (id, food) in told {
                        server.set_food(id, food);
                    }
                });

                for victim in starved {
                    let victim = world.entity_from_id(victim);
                    let at = victim.try_get::<&Position>(|position| position.0);
                    hurt(victim, Damaged {
                        attacker: None,
                        amount: STARVE_DAMAGE,
                        // From where the victim is standing, which resolves to
                        // no impulse at all: starving is the one damage in the
                        // game that must not move anybody, or a player who
                        // stopped fighting would be nudged off the map by their
                        // own empty stomach.
                        knockback: Knockback::from(at.unwrap_or_default()),
                        kind: DamageKind::Environment,
                    });
                }
            }
        });

        // `Smashed` and not `Damaged`: `Damaged` is emitted at every attempt,
        // including the ones `apply_damage` refuses for respawn immunity or a
        // shield, and a hit that was refused is not a hit that fed anybody.
        // `Smashed` goes out only once the health has actually come off.
        world
            .observer_named::<Smashed, ()>("feed_the_attacker")
            .with(Player::id())
            .each_iter(|it, index, ()| {
                let Some(attacker) = it.param().attacker else {
                    return;
                };
                let victim = it.entity(index);
                // Cow's recoil and Spider's lifesteal both name the caster as
                // their own attacker. Hurting yourself is not hitting an enemy.
                if attacker == victim.id() {
                    return;
                }

                let world = it.world();
                let attacker = world.entity_from_id(attacker);
                if !attacker.has(Hunger::id()) {
                    return;
                }
                // Written through the live pointer rather than read, copied and
                // `set` back, which is how `damage.rs` writes `Health` from
                // inside its own observer and for the same stated reason: a
                // `set` from inside an observer is a deferred command, so two
                // hits landing in one frame would both read the same pre-frame
                // bar and both write one point onto it. No test here
                // demonstrates that -- driving two `hurt`s inside
                // `world.defer` fed the attacker twice under both spellings --
                // so this is the idiom the crate already uses rather than a
                // measured fix.
                let (fed, food) =
                    attacker.get::<&mut Hunger>(|hunger| (hunger.feed(FOOD_PER_HIT), hunger.food));
                if !fed {
                    return;
                }

                if let Some(id) = attacker.try_get::<&PlayerId>(|id| *id) {
                    world.get::<&ServerHandle>(|server| server.set_food(id, food));
                }
            });
    }
}

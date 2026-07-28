//! Every ability in the game, driven through the mock seam and held to what its
//! kit declared it does.
//!
//! This file names no kit and no ability. It walks [`ability::manifest`], which
//! is a query over the world rather than a list, so a kit imported from outside
//! the crate is covered the moment its module runs and a kit added tomorrow
//! fails here rather than passing quietly.
//!
//! What this proves and what it does not: this is the game half, so what it
//! reads is the call log of a [`MockServer`] -- the same calls the adapter turns
//! into packets. It says an ability computed the effect it promised. It does not
//! say the packet carrying that effect reaches a client, which is what
//! `nix run .#smash-e2e` is for, and which drives the same manifest.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::{Game, TICK};
use smash::{
    module::{
        ability::{self, Declared, Observable},
        damage::{DamageKind, Damaged, MatchClock, hurt},
        kit::{self, Playing},
        knockback::Knockback,
        player::{Energy, Health, OnGround, Position},
    },
    server::{PlayerId, mock::Call},
};

/// How far in front of the caster the near victim stands.
///
/// Not a whole number, and not the distance any ability centres its blast on.
/// A victim standing exactly on a splash centre is launched away from the point
/// they occupy, which normalises to no direction and no launch, so a round
/// number would make several abilities look like they deal no knockback when
/// the arrangement is what has no answer.
const NEAR: f32 = 3.5;

/// The far victim, for the projectiles that need room to arm.
const FAR: f32 = 8.5;

struct Bench {
    game: Game,
    caster: Entity,
    near: Entity,
    far: Entity,
}

impl Bench {
    fn new() -> Self {
        let mut game = Game::new();
        let caster = game.player("caster", Vec3::ZERO);
        let near = game.player("near", Vec3::new(NEAR, 0.0, 0.0));
        let far = game.player("far", Vec3::new(FAR, 0.0, 0.0));
        Self {
            game,
            caster,
            near,
            far,
        }
    }

    fn caster(&self) -> EntityView<'_> {
        self.game.world.entity_from_id(self.caster)
    }

    /// Put the caster on `kit`, stand everybody back up and clear the log.
    fn reset(&self, kit_name: &str) {
        let world = &self.game.world;
        let chosen =
            kit::by_name(world, kit_name).unwrap_or_else(|| panic!("the registry lost {kit_name}"));

        for entity in [self.caster, self.near, self.far] {
            kit::apply(world, world.entity_from_id(entity), chosen);
        }
        self.place(Vec3::ZERO);
        self.game.world.set(MatchClock(1.0));
        self.game.server.take();
    }

    /// Give the caster the Smash Crystal ability, if this entry needs one.
    fn arm(&self, entry: &Declared) {
        if !entry.ultimate || ability::granted_in_slot(self.caster(), entry.slot).is_some() {
            return;
        }
        assert!(
            kit::grant_ultimate(&self.game.world, self.caster(), 600.0),
            "{} / {}: the Smash Crystal granted nothing",
            entry.kit,
            entry.name
        );
    }

    /// Fire `entry` the way a player would: hold a charge ability for its full
    /// charge time, tap everything else. Does not wait afterwards.
    fn press(&self, entry: &Declared) {
        match entry.charge_time {
            Some(seconds) => {
                ability::use_slot(self.caster(), entry.slot);
                self.game.advance(seconds, 8);
                ability::release_slot(self.caster(), entry.slot);
            }
            None => ability::use_slot(self.caster(), entry.slot),
        }
    }

    /// Long enough for the slowest projectile in the game to cross the far
    /// victim.
    fn settle(&self) {
        self.game.advance(1.5, 30);
    }

    /// Wait out `entry`'s cooldown and set the three of them up again, `attempt`
    /// steps further along the aim axis.
    ///
    /// The whole arrangement moves rather than the caster alone, so every
    /// distance and direction an ability cares about is identical to the first
    /// press and only the absolute position differs. That difference is what
    /// Wither Image needs: it drops a decoy on one press and swaps you to it on
    /// the next, and a swap back to the spot you never left is not a teleport
    /// anybody could see.
    fn recover(&self, entry: &Declared, attempt: u32) {
        self.game.advance(entry.cooldown + 0.5, 30);
        self.place(Vec3::new(
            f32::from(u16::try_from(attempt).unwrap_or(0)) * -4.0,
            0.0,
            0.0,
        ));
    }

    /// Stand all three up, healthy and grounded, `base` blocks from the origin.
    fn place(&self, base: Vec3) {
        let world = &self.game.world;
        for (entity, offset) in [
            (self.caster, Vec3::ZERO),
            (self.near, Vec3::new(NEAR, 0.0, 0.0)),
            (self.far, Vec3::new(FAR, 0.0, 0.0)),
        ] {
            let player = world.entity_from_id(entity);
            player.set(Position(base + offset));
            player.set(OnGround(true));
            player.get::<&mut Health>(|health| health.current = health.max);
        }
        // Every ability that costs energy is used once, so a full bar is the
        // condition under test rather than a coincidence of the previous one.
        if let Some(mut energy) = self.caster().try_get::<&Energy>(|e| *e) {
            energy.current = energy.max;
            self.caster().set(energy);
        }
    }

    fn health_of(&self, entity: Entity) -> Health {
        self.game.world.entity_from_id(entity).cloned::<&Health>()
    }

    fn id_of(&self, entity: Entity) -> PlayerId {
        self.game.world.entity_from_id(entity).cloned::<&PlayerId>()
    }
}

/// Whether the log shows `observable` happening to the right party.
///
/// `held` is the one reading that cannot be taken here: see [`Immediate`].
fn observed(bench: &Bench, observable: Observable, before: &Snapshot, held: bool) -> bool {
    let calls = bench.game.server.calls();
    match observable {
        Observable::HurtsTarget => {
            bench.health_of(bench.near).current < before.near_health
                || bench.health_of(bench.far).current < before.far_health
        }
        Observable::LaunchesTarget => [bench.near, bench.far].iter().any(|victim| {
            bench
                .game
                .server
                .total_velocity(bench.id_of(*victim))
                .length()
                > 1e-4
        }),
        Observable::LaunchesCaster => {
            bench
                .game
                .server
                .total_velocity(bench.id_of(bench.caster))
                .length()
                > 1e-4
        }
        Observable::TeleportsCaster => {
            let caster = bench.id_of(bench.caster);
            calls.iter().any(|call| {
                matches!(call, Call::Teleport(id, to) if *id == caster && to.distance(before.caster_at) > 1.0)
            })
        }
        Observable::HealsCaster => bench.health_of(bench.caster).current > before.caster_health,
        // Measured rather than read off a component: what a player experiences
        // is the next swing hurting more, and a bonus that never reaches the
        // melee path is a bonus that does not exist.
        Observable::BuffsMelee => {
            let clock = bench.game.world.cloned::<&MatchClock>().0;
            let boosted = melee_damage(bench, clock);
            boosted > before.baseline_melee + 1e-3
        }
        // The distinguishing word is *keeps*. Every ability can take health off
        // somebody once, and reading a single drop would pass for all fifty-one
        // of them. So the clock is advanced with nothing cast, and what is
        // measured is health lost during a window in which the ability is over.
        Observable::AfflictsTarget => {
            // Read eagerly, into an array. A lazy iterator here would sample
            // health *after* the wait and compare it against itself, which
            // passes for every ability in the game.
            let watched = [
                (bench.near, bench.health_of(bench.near).current),
                (bench.far, bench.health_of(bench.far).current),
            ];
            bench.game.advance(LINGER_SECONDS, 40);
            watched
                .into_iter()
                .any(|(victim, was)| bench.health_of(victim).current < was - 1e-3)
        }
        // Nothing is cast. The claim is that the ability is still acting, so
        // what is measured is the world changing anyway -- damage landing on
        // anybody, or a launch, or a burst of particles. Broader than
        // `AfflictsTarget`, which is health specifically and on a victim
        // specifically, because an ultimate's mode is not always damage: Giga
        // Slime's is a shield and Frenzy's is a lunge.
        Observable::Sustains => {
            // Everybody is stood back up first. An ultimate that has already
            // killed both victims cannot hurt them again, so without this the
            // *strongest* modes are the ones that read as doing nothing --
            // which is how Arrow Storm first failed this.
            bench.place(before.caster_at);
            let watched =
                [bench.caster, bench.near, bench.far].map(|player| bench.health_of(player).current);
            bench.game.server.take();
            bench.game.advance(LINGER_SECONDS, 40);

            let hurt_somebody = [bench.caster, bench.near, bench.far]
                .iter()
                .zip(watched)
                .any(|(player, was)| bench.health_of(*player).current < was - 1e-3);
            hurt_somebody
                || bench.game.server.calls().iter().any(|call| {
                    matches!(
                        call,
                        Call::AddVelocity(..) | Call::SetHealth(..) | Call::Particles(..)
                    )
                })
        }
        // Both probes were taken either side of the press, in `Immediate`,
        // because neither can be taken from here: see that type.
        Observable::ShieldsCaster => held,
    }
}

/// How long the sweep waits, with nothing cast, to see whether something the
/// ability left behind is still working.
///
/// Longer than the slowest interval any effect in the roster ticks on, which is
/// Spider's poison at 1.25 s. An effect that ticks slower than this and an
/// effect that does not exist look identical from here, so a kit adding one has
/// to raise this with it.
const LINGER_SECONDS: f32 = 2.0;

/// Facts that can only be read either side of the press itself.
///
/// Sky Squid's shield lasts one second and [`Bench::settle`] waits one and a
/// half, so a reading taken after settling finds it already over and calls a
/// working ability broken. Giga Slime's lasts nineteen, so a reading that waits
/// for it to end never gets one. The only window both fit in is the press.
struct Immediate {
    /// The same hit landed before the cast and was refused after it.
    ///
    /// Two probes and not one, because a shield that never lifts and a server
    /// that cannot deal damage produce the same single reading. Taken either
    /// side of the press rather than before and after the *window*, so the
    /// check does not need to know how long the window is -- which is what lets
    /// one shape cover both a one-second shield and a nineteen-second one.
    ///
    /// Only taken when the entry claims [`Observable::ShieldsCaster`]. The probe
    /// costs the caster health and would otherwise perturb every other reading
    /// in the sweep, `heals_caster` most of all.
    held: bool,
}

impl Immediate {
    /// `landed_before` is [`probe_hit`] from before the press. The caller takes
    /// it, because by the time this runs the ability has already fired.
    fn taken(bench: &Bench, entry: &Declared, landed_before: bool) -> Self {
        Self {
            held: entry.proves.contains(&Observable::ShieldsCaster)
                && landed_before
                && !probe_hit(bench),
        }
    }
}

/// Hit the caster for a fixed amount and report whether it landed.
///
/// The amount only has to survive the heaviest armour in the game: Iron Golem's
/// 64% reduction would turn a one-point probe into 0.36 and a rounding argument
/// into a test failure.
fn probe_hit(bench: &Bench) -> bool {
    let caster = bench.caster();
    let before = bench.health_of(bench.caster).current;
    hurt(caster, Damaged {
        attacker: Some(bench.near),
        amount: 4.0,
        knockback: Knockback::from(Vec3::ZERO).times(0.0),
        kind: DamageKind::Ability,
    });
    bench.health_of(bench.caster).current < before - 1e-3
}

/// What one melee swing at the near victim takes off, right now.
fn melee_damage(bench: &Bench, now: f32) -> f32 {
    use smash::module::{damage::MeleeBonus, kit::KitStats};

    let caster = bench.caster();
    let base = caster
        .target(Playing, 0)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| stats.melee_damage))
        .unwrap_or(1.0);
    let bonus = caster
        .try_get::<&MeleeBonus>(|bonus| bonus.applies_to(bench.near, now))
        .unwrap_or(0.0);

    let victim = bench.game.world.entity_from_id(bench.near);
    let before = victim.cloned::<&Health>().current;
    hurt(victim, Damaged {
        attacker: Some(bench.caster),
        amount: base + bonus,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });
    before - victim.cloned::<&Health>().current
}

struct Snapshot {
    near_health: f32,
    far_health: f32,
    caster_health: f32,
    caster_at: Vec3,
    baseline_melee: f32,
}

/// Everything the checks below compare against, taken immediately before a
/// press.
///
/// The caster is deliberately left short of full health. `SetHealth` is scaled
/// to twenty on the wire, so a heal that also raises the maximum -- which is
/// what Mooshroom Madness is -- lands on exactly the same number a full-health
/// player was already showing, and would read as nothing happening at all.
fn snapshot(bench: &Bench) -> Snapshot {
    bench
        .game
        .world
        .entity_from_id(bench.caster)
        .get::<&mut Health>(|health| health.current = health.max * 0.5);

    let clock = bench.game.world.cloned::<&MatchClock>().0;
    let baseline_melee = melee_damage(bench, clock);
    // Undo the probe swing so the ability under test sees a full-health victim.
    let victim = bench.game.world.entity_from_id(bench.near);
    victim.get::<&mut Health>(|health| health.current = health.max);
    bench.game.server.take();

    Snapshot {
        near_health: bench.health_of(bench.near).current,
        far_health: bench.health_of(bench.far).current,
        caster_health: bench.health_of(bench.caster).current,
        caster_at: bench
            .game
            .world
            .entity_from_id(bench.caster)
            .cloned::<&Position>()
            .0,
        baseline_melee,
    }
}

/// The guard that makes the rest of this file mandatory rather than optional.
///
/// An ability with an empty declaration would sail through every check below,
/// because there would be nothing to check. Refusing it here is what turns
/// "adding a kit means adding data" into "adding a kit means saying what the
/// data does".
#[test]
fn every_ability_declares_what_a_client_would_see() {
    let game = Game::new();
    let manifest = ability::manifest(&game.world);

    assert!(
        manifest.len() >= 50,
        "the registry only found {} abilities, which is fewer than the roster has; something is \
         not being discovered",
        manifest.len()
    );

    let silent: Vec<_> = manifest
        .iter()
        .filter(|entry| entry.proves.is_empty())
        .map(|entry| format!("{} / {}", entry.kit, entry.name))
        .collect();
    assert!(
        silent.is_empty(),
        "these abilities declare no observable effect, so no gate can test them: {silent:?}"
    );
}

/// How many times an ability may be pressed before it has to have done what it
/// said.
///
/// More than one because two abilities in the roster are stateful across uses:
/// Wither Image drops a decoy on the first press and swaps you to it on the
/// second, and that is the ability, not a workaround. A press that never
/// achieves anything still fails, however many it gets.
const ATTEMPTS: u32 = 3;

/// The sweep: every declared ability, fired, and every observation it declared,
/// checked.
#[test]
fn every_declared_effect_actually_happens() {
    let manifest = ability::manifest(&Game::new().world);
    let mut failures = Vec::new();

    for entry in manifest {
        let bench = Bench::new();
        bench.reset(entry.kit);
        let mut outstanding: Vec<Observable> = entry.proves.to_vec();

        for attempt in 0..ATTEMPTS {
            if outstanding.is_empty() {
                break;
            }
            if attempt > 0 {
                // Wait the cooldown out and stand everybody back up, so the
                // next press starts from the same conditions as the first.
                bench.recover(&entry, attempt);
            }
            bench.arm(&entry);
            // Before the press, so the pair brackets it. See `Immediate::held`.
            let landed_before =
                entry.proves.contains(&Observable::ShieldsCaster) && probe_hit(&bench);
            let before = snapshot(&bench);
            bench.press(&entry);
            // One tick, because a real client's probe cannot arrive sooner
            // than the next one. Reading with zero ticks elapsed is the one
            // thing a scripted client can never do, and it is what let this
            // check pass while `nix run .#smash-e2e` failed on the same
            // commit: a shield that exists for exactly zero ticks satisfied
            // the bench and nothing else.
            bench.game.advance(TICK, 1);
            let immediate = Immediate::taken(&bench, &entry, landed_before);
            bench.settle();
            outstanding
                .retain(|observable| !observed(&bench, *observable, &before, immediate.held));
        }

        for observable in outstanding {
            failures.push(format!(
                "{} / {} declares {} and did not do it",
                entry.kit,
                entry.name,
                observable.as_str()
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The declaration is exhaustive for movement, not just a lower bound.
///
/// An ability that launches somebody without saying so is the same defect as one
/// that says it launches and does not: the registry stops describing the game.
/// It is also where the quiet bugs live. Blaze's Inferno is documented as having
/// no knockback and was moving everyone it touched a fifth of a block a tick,
/// because the horizontal floor in the knockback model applied even to a hit
/// whose multiplier was zero.
///
/// Damage is deliberately not checked this way: several abilities cost the
/// caster health on purpose, and "hurts somebody" has no single subject.
#[test]
fn nothing_moves_that_did_not_say_it_would() {
    let manifest = ability::manifest(&Game::new().world);
    let mut failures = Vec::new();

    for entry in manifest {
        let bench = Bench::new();
        bench.reset(entry.kit);
        bench.arm(&entry);
        let before = snapshot(&bench);
        bench.press(&entry);
        bench.settle();

        for observable in [
            Observable::LaunchesTarget,
            Observable::LaunchesCaster,
            Observable::TeleportsCaster,
        ] {
            // `held` is false: this test asks only about movement, and no
            // movement observation reads it.
            if !entry.proves.contains(&observable) && observed(&bench, observable, &before, false) {
                failures.push(format!(
                    "{} / {} does not declare {} and did it anyway",
                    entry.kit,
                    entry.name,
                    observable.as_str()
                ));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A cooldown that does not refuse the next use is a cooldown a player cannot
/// feel, and an ability that says it refunds on a hit and does not is a kit
/// whose whole selling point is missing. Both come from the registry.
#[test]
fn a_cooldown_does_what_the_registry_says_it_does() {
    let manifest = ability::manifest(&Game::new().world);
    let mut failures = Vec::new();

    for entry in manifest {
        // Barrage deliberately has none at all.
        if entry.cooldown <= 0.0 {
            continue;
        }
        let bench = Bench::new();
        bench.reset(entry.kit);
        bench.arm(&entry);
        bench.press(&entry);

        let immediately = ability::activate(bench.caster(), entry.slot, 1.0);
        if immediately != Err(ability::Refusal::OnCooldown) {
            failures.push(format!(
                "{} / {} has a {}s cooldown and answered {immediately:?} to a second use in the \
                 same tick",
                entry.kit, entry.name, entry.cooldown
            ));
            continue;
        }

        // The refund is a claim in the registry, so it is checked rather than
        // excused: fly the projectile into the victim and the cooldown should
        // be gone.
        if entry.refunds_on_hit {
            bench.settle();
            let after_hit = ability::activate(bench.caster(), entry.slot, 1.0);
            if after_hit != Ok(()) {
                failures.push(format!(
                    "{} / {} says it refunds on a hit; after one landed it answered {after_hit:?}",
                    entry.kit, entry.name
                ));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Changing kit while holding a crystal must not cost you the new kit's
/// abilities.
///
/// The expiry sweep takes a grant back by unlinking it from whoever holds it. A
/// kit change destroys the granted instance first, and flecs recycles entity
/// ids, so an expiry that fires afterwards and trusts a stale id can unlink an
/// ability that merely inherited the number. The symptom is the worst kind:
/// right-clicking the slot does nothing at all and says nothing at all, because
/// an empty slot is not an error.
#[test]
fn a_crystal_left_over_from_a_previous_kit_does_not_eat_the_next_one() {
    let bench = Bench::new();
    bench.reset("Guardian");
    assert!(kit::grant_ultimate(&bench.game.world, bench.caster(), 5.0));

    bench.reset("Iron Golem");
    // Past when the old grant would have lapsed.
    bench.game.advance(8.0, 80);

    let slots: Vec<u8> = ability::manifest(&bench.game.world)
        .into_iter()
        .filter(|entry| entry.kit == "Iron Golem" && !entry.ultimate)
        .map(|entry| entry.slot)
        .collect();
    for slot in slots {
        assert!(
            ability::granted_in_slot(bench.caster(), slot).is_some(),
            "slot {slot} lost its ability when a previous kit's crystal lapsed"
        );
    }
}

/// The Smash Crystal's window, from the outside: granted, in the hotbar, gone
/// again when it lapses.
#[test]
fn an_ultimate_is_granted_for_a_window_and_then_taken_back() {
    let bench = Bench::new();
    bench.reset("Iron Golem");

    let ultimate = kit::ultimate_name(bench.caster()).expect("Iron Golem declares an ultimate");
    assert!(kit::grant_ultimate(&bench.game.world, bench.caster(), 5.0));
    assert!(
        kit::hotbar(bench.caster())
            .iter()
            .any(|item| item.name == ultimate),
        "the granted ultimate never reached the hotbar"
    );
    assert!(
        !kit::grant_ultimate(&bench.game.world, bench.caster(), 5.0),
        "a second crystal should not stack"
    );

    bench.game.advance(6.0, 60);
    assert!(
        !kit::hotbar(bench.caster())
            .iter()
            .any(|item| item.name == ultimate),
        "the ultimate outlived its window"
    );
    assert_eq!(
        ability::activate(bench.caster(), 8, 1.0),
        Ok(()),
        "an expired ultimate should be nothing at all in that slot, not a refusal"
    );
}

/// A shield is up before anybody can hit you, including when the cast is
/// deferred.
///
/// This is the test the sweep could not be. `every_declared_effect_actually_happens`
/// drives `ability::use_slot` from test code, which is *outside* any flecs
/// system, so every `add` an ability makes lands immediately and the sweep sees
/// a world nobody in production ever sees. On a real server the press arrives as
/// a packet, `smash::on_item_interact` drains it from inside a system, and every
/// mutation the payload makes is queued until the end of the frame.
///
/// `smash::on_attack` is registered in the same pipeline, after it. So a client
/// that swings in the same tick as it right-clicks is answered by a damage
/// observer that cannot see the shield the press just armed, and the hit lands
/// through a window the ability says is closed. `nix run .#smash-e2e` failed on
/// exactly that while `cargo nextest run -p smash` was 259 for 259.
///
/// `defer_begin`/`defer_end` is that condition, in-process: it is the same
/// suspension flecs puts a system body under.
#[test]
fn a_shield_is_up_before_anybody_can_swing() {
    let manifest = ability::manifest(&Game::new().world);
    let shields: Vec<_> = manifest
        .into_iter()
        .filter(|entry| entry.proves.contains(&Observable::ShieldsCaster))
        .collect();
    assert!(
        !shields.is_empty(),
        "no ability declares a shield, so this test is checking nothing"
    );

    let mut failures = Vec::new();
    for entry in shields {
        let bench = Bench::new();
        bench.reset(entry.kit);
        bench.arm(&entry);

        assert!(
            probe_hit(&bench),
            "{} / {}: the probe has to land before the cast, or this proves nothing",
            entry.kit,
            entry.name
        );
        bench.place(Vec3::ZERO);

        // Everything between `defer_begin` and `defer_end` is one frame, which
        // is exactly the suspension a system body runs under.
        bench.game.world.defer_begin();
        let fired = ability::activate(bench.caster(), entry.slot, 1.0);
        let landed_inside_the_frame = probe_hit(&bench);
        bench.game.world.defer_end();

        assert_eq!(
            fired,
            Ok(()),
            "{} / {} refused to fire, so nothing was armed",
            entry.kit,
            entry.name
        );
        if landed_inside_the_frame {
            failures.push(format!(
                "{} / {} armed its shield and a swing in the same frame still landed: the arm was \
                 deferred and the damage observer could not see it",
                entry.kit, entry.name
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// No ultimate leaves a player permanently changed.
///
/// A Smash Crystal is a window. Anything an ultimate alters about the player
/// themselves -- their maximum health, their armour, how far they are thrown --
/// has to be back where it started once the window closes, or picking a crystal
/// up twice in a match compounds and the kit drifts away from every number the
/// registry publishes.
///
/// Written as a sweep over the class rather than as a test for the one kit that
/// had the bug. Cow's Mooshroom Madness raised the maximum by five hearts and
/// never lowered it, so a Cow with two crystals finished on forty and a third
/// on fifty; the reason to check every ultimate instead is that nothing about
/// that bug was specific to Cow, and the next one will not be either.
#[test]
fn an_ultimate_gives_the_player_back_when_it_is_done() {
    use smash::module::{damage::Armor, kit::KitStats, knockback::KnockbackTaken};

    let manifest = ability::manifest(&Game::new().world);
    let mut failures = Vec::new();

    for entry in manifest.into_iter().filter(|entry| entry.ultimate) {
        let bench = Bench::new();
        bench.reset(entry.kit);

        let stats = bench
            .caster()
            .target(Playing, 0)
            .and_then(|kit| kit.try_get::<&KitStats>(|stats| *stats))
            .expect("a player on a kit has its stats");

        bench.arm(&entry);
        bench.press(&entry);

        // Past the longest window any ultimate declares, with room for the
        // final beat and the teardown that follows it.
        bench.game.advance(ability::ULTIMATE_SECONDS + 4.0, 240);

        let health = bench.health_of(bench.caster);
        let armor = bench.caster().cloned::<&Armor>().0;
        let knockback = bench.caster().cloned::<&KnockbackTaken>().0;

        let label = format!("{} / {}", entry.kit, entry.name);
        if (health.max - stats.max_health).abs() > 1e-3 {
            failures.push(format!(
                "{label} left the caster on a maximum of {} health where the kit declares {}",
                health.max, stats.max_health
            ));
        }
        if (armor - stats.armor).abs() > 1e-3 {
            failures.push(format!(
                "{label} left the caster on {armor} armour where the kit declares {}",
                stats.armor
            ));
        }
        if (knockback - stats.knockback_taken).abs() > 1e-3 {
            failures.push(format!(
                "{label} left the caster taking {knockback}x knockback where the kit declares {}",
                stats.knockback_taken
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What the charge accumulator hands a payload, against wall clock.
///
/// Seven abilities in the roster are hold-and-release, and every one of them
/// scales off `Cast::charge`. If the accumulator under-counts, all seven fire
/// weaker than they declare and the two that gate on a threshold -- Slime's
/// rocket size and Skeleton's arrow count -- reach it late or never, which is
/// a bug that presents as "this kit feels bad" and as nothing else.
///
/// Measured through the payload rather than by reading `Charging::held`,
/// because what matters is the number the ability is given.
mod charge {
    use std::sync::atomic::{AtomicU32, Ordering};

    use glam::Vec3;
    use smash::module::{
        ability::{self, Cast, Observable},
        kit::{self, AbilitySpec, KitStats},
        player::Position,
    };

    use super::harness::Game;

    /// The last charge a payload was handed, as bits. An atomic because the
    /// payload is a bare `fn` with nowhere to put a closure capture, which is
    /// the same reason `OnActivate` is a `fn` in the first place.
    static SEEN: AtomicU32 = AtomicU32::new(0);

    fn record(cast: &Cast<'_>) {
        SEEN.store(cast.charge.to_bits(), Ordering::SeqCst);
    }

    /// Seconds to full charge for the ability under test. Arbitrary, and
    /// deliberately not any real kit's, so the test measures the accumulator
    /// rather than a kit's tuning.
    const FULL: f32 = 2.0;

    fn charge_after(held_for: f32, steps: u32) -> f32 {
        let mut game = Game::new();
        kit::define(&game.world, "ChargeProbe", KitStats::default())
            .ability(AbilitySpec {
                name: "Probe",
                sound: "minecraft:entity.arrow.shoot",
                description: "Held for a known time, and reports what it was given.",
                charge_time: Some(FULL),
                proves: &[Observable::HurtsTarget],
                activate: record,
                ..AbilitySpec::DEFAULT
            })
            .register();

        let player = game.player("holder", Vec3::ZERO);
        let player = game.world.entity_from_id(player);
        player.set(Position(Vec3::ZERO));
        let probe = kit::by_name(&game.world, "ChargeProbe").expect("just defined");
        kit::apply(&game.world, player, probe);

        SEEN.store(0, Ordering::SeqCst);
        ability::use_slot(player, 0);
        game.advance(held_for, steps);
        ability::release_slot(player, 0);
        f32::from_bits(SEEN.load(Ordering::SeqCst))
    }

    /// Holding for the full charge time hands the payload 1.0, and holding for
    /// half of it hands over a half.
    ///
    /// The tolerance is one tick's worth. `Charging` is created by the observer
    /// that handles the press and first ticked by the system on the frame
    /// after, so a hold measured in whole ticks is short by at most one of
    /// them, and asserting exactness would be asserting an ordering the game
    /// does not have.
    #[test]
    fn a_hold_is_worth_its_wall_clock() {
        let tolerance = 1.0 / 20.0 / FULL + 1e-3;

        let full = charge_after(FULL, 40);
        assert!(
            (full - 1.0).abs() <= tolerance,
            "holding for the whole {FULL}s charge time was worth {full}, not 1.0"
        );

        let half = charge_after(FULL / 2.0, 20);
        assert!(
            (half - 0.5).abs() <= tolerance,
            "holding for half the {FULL}s charge time was worth {half}, not 0.5"
        );
    }

    /// And the step function the two threshold abilities use agrees with it.
    ///
    /// Barrage is `1 + charge_steps(charge, 4)`, so a full hold has to be five
    /// arrows and not four or three. Checked here against the accumulator
    /// rather than in isolation, because the two being individually defensible
    /// and jointly wrong is exactly how a five-arrow ability fires three.
    #[test]
    fn a_full_hold_reaches_the_top_step() {
        let full = charge_after(FULL, 40);
        assert_eq!(
            1 + ability::charge_steps(full, 4),
            5,
            "a full hold was worth {full}, which is {} arrows and not five",
            1 + ability::charge_steps(full, 4)
        );
    }
}

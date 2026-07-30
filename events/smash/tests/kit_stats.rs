//! Every field of [`KitStats`] has to be able to change the game.
//!
//! The gate is one test per field, and each one is the same shape: two players
//! on kits that are byte-for-byte identical except for that one field, the same
//! scenario run against both, and a difference required in what a client would
//! see at the end of it. A field that cannot be made to matter is a field with
//! no implementation, and it fails here.
//!
//! This is deliberately stronger than "every field has a reader", which
//! `let _ = stats.regen;` satisfies. ENG-11450 is what it is for: `regen`,
//! `hunger_interval`, `jump_power` and `jump_control` were set by every one of
//! sixteen kits, several of them tuned against the wiki and argued about in
//! comments, and read by nothing at all. Nothing failed, because nothing asked
//! this question.
//!
//! The five fields that already worked are gated too, and they are not filler:
//! they are the evidence that the harness below can tell a difference when
//! there is one. A gate whose only passing cases are the ones written alongside
//! the feature is a gate nobody has watched work.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        damage::{DamageKind, Damaged, hurt},
        kit::{self, KitStats},
        knockback::Knockback,
        lobby::{Lobby, Phase},
        player::{Energy, Health, OnGround, Position},
        vitals::Hunger,
    },
    server::{PlayerId, mock::Call},
};

/// Where the two subjects and their punching bags stand.
///
/// Well above the default arena's kill plane, because the gate runs in
/// [`Phase::Playing`] and that is the phase the floor is armed in. Four
/// separate columns so no scenario's knockback or splash reaches a player it
/// was not aimed at.
const COLUMNS: [Vec3; 4] = [
    Vec3::new(0.0, 200.0, 0.0),
    Vec3::new(0.0, 200.0, 60.0),
    Vec3::new(120.0, 200.0, 0.0),
    Vec3::new(120.0, 200.0, 60.0),
];

/// The stats both kits are built from.
///
/// Every number is off the default rather than off a real kit, so a test that
/// perturbs one field is perturbing exactly one thing and the reader does not
/// have to hold a kit's table in their head to see it.
fn base() -> KitStats {
    KitStats {
        // A bar on both kits by default, so the `energy` test perturbs a
        // number rather than the presence of the component. Its own test says
        // what it changes.
        energy: Some((1.0, 0.1)),
        ..KitStats::default()
    }
}

/// One half of the differential: a player on one of the two kits, and an
/// identical opponent for them to hit.
struct Side {
    player: Entity,
    foe: Entity,
}

/// Two players whose kits differ in exactly one [`KitStats`] field.
struct Gate {
    game: Game,
    /// The player on the unmodified kit, first.
    sides: [Side; 2],
}

impl Gate {
    /// Build the world. `mutate` is the single field under test.
    ///
    /// Both subjects and both opponents are in one world and one run, so a
    /// difference between them cannot be run-to-run noise: they share a tick
    /// count, a random-free simulation and the same lobby phase.
    fn new(mutate: impl FnOnce(&mut KitStats)) -> Self {
        let mut game = Game::new();

        let mut variant = base();
        mutate(&mut variant);
        assert_ne!(
            variant,
            base(),
            "this test perturbs no field, so it would pass against any implementation"
        );

        kit::define(&game.world, "GateBase", base()).register();
        kit::define(&game.world, "GateVariant", variant).register();

        let names = ["base", "base_foe", "variant", "variant_foe"];
        let spawned: Vec<Entity> = names
            .iter()
            .zip(COLUMNS)
            .map(|(name, at)| game.player(name, at))
            .collect();

        // Both opponents are on the base kit whichever side they stand on. The
        // only difference in the world is the one field.
        for (index, entity) in spawned.iter().enumerate() {
            let on = if index == 2 {
                "GateVariant"
            } else {
                "GateBase"
            };
            let chosen = kit::by_name(&game.world, on).expect("just defined");
            kit::apply(&game.world, game.world.entity_from_id(*entity), chosen);
        }

        // The clocks under test only run during a match, so the gate runs
        // during one. Set rather than waited into: the transitions scatter
        // players onto spawn points, and four players in one place is not the
        // scenario any of these tests set up.
        game.world.set(Lobby {
            phase: Phase::Playing,
            timer: 0.0,
        });

        Self {
            game,
            sides: [
                Side {
                    player: spawned[0],
                    foe: spawned[1],
                },
                Side {
                    player: spawned[2],
                    foe: spawned[3],
                },
            ],
        }
    }

    fn view(&self, entity: Entity) -> EntityView<'_> {
        self.game.world.entity_from_id(entity)
    }

    fn id(&self, entity: Entity) -> PlayerId {
        self.view(entity).cloned::<&PlayerId>()
    }

    /// Do the same thing to each side before any of them is read.
    ///
    /// Acting on both and only then advancing is what keeps the two sides
    /// symmetrical: they share one world, so reading one before acting on the
    /// other would give the first side a head start no assertion could tell
    /// apart from the field working.
    fn each(&self, act: impl Fn(&Self, &Side)) {
        for side in &self.sides {
            act(self, side);
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a scenario is a positive number of seconds, and the longest one here is 16"
    )]
    fn advance(&self, seconds: f32) {
        self.game
            .advance(seconds, (seconds / harness::TICK).round() as u32);
    }

    /// The gate itself: read `what` off each side and require the two to
    /// disagree.
    fn differ<T: PartialEq + std::fmt::Debug>(&self, what: &str, read: impl Fn(&Self, &Side) -> T) {
        let base = read(self, &self.sides[0]);
        let variant = read(self, &self.sides[1]);
        assert_ne!(
            base, variant,
            "the two kits differ in exactly one KitStats field and {what} came out the same, so \
             nothing in the game reads that field"
        );
    }

    /// Hurt `player` for `amount`, from a fixed direction so the knockback is
    /// the same shape on both sides.
    fn hit(&self, victim: Entity, amount: f32, kind: DamageKind) {
        let victim = self.view(victim);
        let from = victim.cloned::<&Position>().0 - Vec3::X;
        hurt(victim, Damaged {
            attacker: None,
            amount,
            knockback: Knockback::from(from),
            kind,
        });
    }

    fn health(&self, entity: Entity) -> Health {
        self.view(entity).cloned::<&Health>()
    }
}

/// A float at the precision anybody could act on: three decimals.
///
/// Requiring a difference at full `f32` precision would let a field that moves
/// the twentieth bit of a result count as implemented, and floats do not
/// implement `Eq` anyway. Three decimals of a health point is a thousandth of a
/// half-heart, which is far finer than the game or the client can draw and far
/// coarser than rounding noise.
#[expect(
    clippy::cast_possible_truncation,
    reason = "health, energy and an impulse are all far inside i64 once scaled by a thousand"
)]
fn observable(value: f32) -> i64 {
    (f64::from(value) * 1000.0).round() as i64
}

fn observable_vec(value: Vec3) -> [i64; 3] {
    [
        observable(value.x),
        observable(value.y),
        observable(value.z),
    ]
}

// ---------------------------------------------------------------------------
// The nine fields.
// ---------------------------------------------------------------------------

/// `melee_damage`: a swing takes off the attacker's kit's number.
#[test]
fn melee_damage_changes_what_a_swing_takes_off() {
    let gate = Gate::new(|stats| stats.melee_damage = base().melee_damage * 2.0);

    // Through the game's own melee function and not a number this test picked,
    // so a `melee_damage` that stopped reading the kit is a failure here rather
    // than a formula compared against a copy of itself.
    gate.each(|gate, side| {
        let clock = gate
            .game
            .world
            .cloned::<&smash::module::damage::MatchClock>()
            .0;
        let amount = smash::input::melee_damage(gate.view(side.player), side.foe, clock);
        gate.hit(side.foe, amount, DamageKind::Melee);
    });
    gate.advance(0.05);

    gate.differ("the health left on the player who was hit", |gate, side| {
        observable(gate.health(side.foe).current)
    });
}

/// `armor`: the same hit lands for less on the better-armoured kit.
#[test]
fn armor_changes_how_much_of_a_hit_lands() {
    let gate = Gate::new(|stats| stats.armor = base().armor + 8.0);

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Melee));
    gate.advance(0.05);

    gate.differ("the health left after an identical hit", |gate, side| {
        observable(gate.health(side.player).current)
    });
}

/// `knockback_taken`: the same hit launches the lighter kit further.
#[test]
fn knockback_taken_changes_how_far_a_hit_launches() {
    let gate = Gate::new(|stats| stats.knockback_taken = base().knockback_taken * 1.6);

    gate.each(|gate, side| gate.hit(side.player, 8.0, DamageKind::Melee));
    gate.advance(0.05);

    // The seam call and not a component: an impulse the game computed and never
    // sent is a bug a state assertion would miss.
    gate.differ("the impulse the server was told to apply", |gate, side| {
        observable_vec(gate.game.server.total_velocity(gate.id(side.player)))
    });
}

/// `max_health`: the bigger pool survives a hit the smaller one does not.
#[test]
fn max_health_changes_how_much_damage_a_kit_survives() {
    let gate = Gate::new(|stats| stats.max_health = base().max_health * 2.0);

    gate.each(|gate, side| gate.hit(side.player, base().max_health, DamageKind::Ability));
    gate.advance(0.05);

    gate.differ(
        "the health left after a hit for a full base bar",
        |gate, side| observable(gate.health(side.player).current),
    );
}

/// `energy`: the bar refills at the kit's rate.
#[test]
fn energy_changes_the_size_and_refill_rate_of_the_bar() {
    let gate = Gate::new(|stats| stats.energy = Some((3.0, 0.6)));

    // Emptied first, because both bars start full and a full bar hides a regen
    // rate completely.
    gate.each(|gate, side| {
        gate.view(side.player)
            .get::<&mut Energy>(|energy| energy.current = 0.0);
    });
    gate.advance(1.0);

    gate.differ(
        "the energy a second of regeneration put back",
        |gate, side| observable(gate.view(side.player).cloned::<&Energy>().current),
    );
}

/// `regen`: health comes back at the kit's rate. ENG-11450.
#[test]
fn regen_changes_how_fast_health_comes_back() {
    let gate = Gate::new(|stats| stats.regen = base().regen * 3.0);

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Ability));
    gate.advance(4.0);

    gate.differ(
        "the health four seconds of regeneration put back",
        |gate, side| observable(gate.health(side.player).current),
    );
}

/// `hunger_interval`: the food bar drains at the kit's rate. ENG-11450.
#[test]
fn hunger_interval_changes_how_fast_the_food_bar_drains() {
    let gate = Gate::new(|stats| stats.hunger_interval = base().hunger_interval / 4.0);

    gate.advance(16.0);

    gate.differ("the food bar after sixteen seconds", |gate, side| {
        gate.view(side.player).cloned::<&Hunger>().food
    });
}

/// What ENG-11440 has to make happen.
///
/// There is no entry point for a double jump in the game at all today -- the
/// press is not routed, `JumpsLeft` is re-armed and never spent, and no
/// velocity is applied -- so there is nothing this function can call. It is the
/// one line ENG-11440 replaces, and the two tests below are `#[ignore]` until
/// it does.
fn double_jump(gate: &Gate, side: &Side) {
    // Airborne, which is the only precondition the game already models.
    gate.view(side.player).set(OnGround(false));
}

/// `jump_power`: the double jump's impulse.
///
/// `#[ignore]`: fails until ENG-11440 lands, because nothing applies the
/// impulse. That failure *is* the finding -- do not delete the test to make the
/// suite green.
#[test]
#[ignore = "ENG-11440: double jump is not implemented; nothing reads jump_power"]
fn jump_power_changes_how_high_a_double_jump_goes() {
    let gate = Gate::new(|stats| stats.jump_power = base().jump_power * 2.0);

    gate.each(double_jump);
    gate.advance(0.1);

    gate.differ("the impulse the double jump asked for", |gate, side| {
        observable_vec(gate.game.server.total_velocity(gate.id(side.player)))
    });
}

/// `jump_control`: whether the double jump goes where you look.
///
/// `#[ignore]`: see [`jump_power_changes_how_high_a_double_jump_goes`].
#[test]
#[ignore = "ENG-11440: double jump is not implemented; nothing reads jump_control"]
fn jump_control_changes_which_way_a_double_jump_goes() {
    let gate = Gate::new(|stats| stats.jump_control = !base().jump_control);

    // Looking sideways, so a controlled jump and an uncontrolled one point in
    // measurably different directions rather than in the same one.
    gate.each(|gate, side| {
        gate.view(side.player)
            .set(smash::module::player::Facing(Vec3::X));
        double_jump(gate, side);
    });
    gate.advance(0.1);

    gate.differ("the direction the double jump asked for", |gate, side| {
        observable_vec(gate.game.server.total_velocity(gate.id(side.player)))
    });
}

// ---------------------------------------------------------------------------
// The part that closes the class.
// ---------------------------------------------------------------------------

/// Every field of [`KitStats`] is named by a test in this file.
///
/// Two halves, and neither works alone. The destructure is exhaustive, so
/// adding a tenth field to `KitStats` stops this file *compiling* until
/// somebody names it -- there is no way to add a stat and quietly not gate it.
/// The source scan then requires that the name it was given here actually
/// appears in a test above, so naming it in the list without writing the test
/// fails too.
///
/// Without both, ENG-11450 recurs: a field arrives, every kit sets it, and no
/// check anywhere notices that nothing reads it.
#[test]
fn every_kit_stats_field_is_gated_by_a_test_in_this_file() {
    const FIELDS: [&str; 9] = [
        "melee_damage",
        "armor",
        "knockback_taken",
        "regen",
        "hunger_interval",
        "max_health",
        "jump_power",
        "jump_control",
        "energy",
    ];

    let KitStats {
        melee_damage: _,
        armor: _,
        knockback_taken: _,
        regen: _,
        hunger_interval: _,
        max_health: _,
        jump_power: _,
        jump_control: _,
        energy: _,
    } = KitStats::default();

    let source = include_str!("kit_stats.rs");
    let gated: Vec<&str> = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn "))
        .filter_map(|line| line.split_once('('))
        .map(|(name, _)| name)
        .collect();

    let missing: Vec<&str> = FIELDS
        .into_iter()
        .filter(|field| !gated.iter().any(|name| name.starts_with(field)))
        .collect();
    assert!(
        missing.is_empty(),
        "these KitStats fields have no gate in this file: {missing:?}. Each needs a test giving \
         two players kits that differ only in it and requiring an observable difference."
    );
}

/// The gate can fail.
///
/// A differential harness that reported a difference between two identical
/// worlds would pass every test above for any implementation, including no
/// implementation. [`Gate::new`] refuses a mutation that changes nothing, which
/// is the same guard from the other end; this is the one that watches a real
/// read come out equal.
#[test]
#[should_panic(expected = "nothing in the game reads that field")]
fn two_identical_kits_fail_the_gate() {
    // A real perturbation, so `Gate::new` is satisfied, read through a quantity
    // that field cannot touch: `armor` changes how much of a hit lands and
    // nothing about the food bar.
    let gate = Gate::new(|stats| stats.armor = base().armor + 8.0);
    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Melee));
    gate.advance(1.0);
    gate.differ("the food bar", |gate, side| {
        gate.view(side.player).cloned::<&Hunger>().food
    });
}

/// Starving is what an empty food bar is for.
///
/// The differential above proves `hunger_interval` reaches the bar. This is the
/// other half of the mechanic and the reason the field exists at all:
/// `[SOURCE]` + `[changelog]` "at zero it deals half a heart per second as true
/// damage", which is Mineplex's replacement for a sudden-death timer.
#[test]
fn an_empty_food_bar_starves_a_player_regardless_of_armour() {
    let gate = Gate::new(|stats| stats.armor = base().armor + 8.0);

    gate.each(|gate, side| {
        gate.view(side.player).set(Hunger {
            food: 0,
            interval: base().hunger_interval,
            elapsed: 0.0,
        });
    });
    let before = gate.health(gate.sides[0].player).current;
    gate.advance(2.0);

    let base_side = gate.health(gate.sides[0].player).current;
    let armoured = gate.health(gate.sides[1].player).current;
    assert!(
        base_side < before,
        "an empty food bar did not starve anybody"
    );
    assert_eq!(
        observable(base_side),
        observable(armoured),
        "hunger is true damage, so armour must not change how fast a player starves"
    );
}

/// Landing a hit puts food back, which is the counter-pressure the drain is
/// there to create.
///
/// `[WIKI]` "If you do not attack, your hunger bar will start depleting, which
/// can be filled back up by hitting other mobs with melee or your special
/// skills."
#[test]
fn landing_a_hit_feeds_the_attacker() {
    let gate = Gate::new(|stats| stats.armor = base().armor + 8.0);
    let side = &gate.sides[0];

    let hungry = Hunger {
        food: 4,
        interval: base().hunger_interval,
        elapsed: 0.0,
    };
    gate.view(side.player).set(hungry);

    let victim = gate.view(side.foe);
    hurt(victim, Damaged {
        attacker: Some(side.player),
        amount: 5.0,
        knockback: Knockback::from(victim.cloned::<&Position>().0 - Vec3::X),
        kind: DamageKind::Melee,
    });

    assert_eq!(
        gate.view(side.player).cloned::<&Hunger>().food,
        hungry.food + smash::module::vitals::FOOD_PER_HIT,
        "landing a hit did not feed the attacker"
    );
    assert!(
        gate.game
            .server
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SetFood(id, _) if *id == gate.id(side.player))),
        "the attacker's client was never told their food bar had moved"
    );
}

/// Neither clock runs outside a match.
///
/// A food bar that drained while a lobby waited for a second player would have
/// somebody starving on the spawn platform before the countdown started.
#[test]
fn the_two_clocks_are_off_in_the_hub() {
    let gate = Gate::new(|stats| stats.regen = base().regen * 3.0);
    gate.game.world.set(Lobby {
        phase: Phase::Waiting,
        timer: 0.0,
    });

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Ability));
    let hurt_to = gate.health(gate.sides[0].player).current;
    gate.advance(16.0);

    assert_eq!(
        observable(gate.health(gate.sides[0].player).current),
        observable(hurt_to),
        "health regenerated in the hub"
    );
    assert_eq!(
        gate.view(gate.sides[0].player).cloned::<&Hunger>().food,
        smash::module::vitals::FULL,
        "the food bar drained in the hub"
    );
}

/// A player on zero health is not healed back over the line.
///
/// `arena::bounds_checks` eliminates anybody `Health::is_dead` and reads that
/// within the same tick as the regeneration system runs. A heal off zero would
/// therefore cancel deaths on some ticks and not others, which is the worst
/// kind of bug to be handed: a kill that sometimes does not count.
#[test]
fn regeneration_does_not_resurrect_a_player_on_zero_health() {
    let gate = Gate::new(|stats| stats.regen = 5.0);
    let subject = gate.sides[1].player;

    // Queued to respawn, which is what a real corpse is, and what puts them out
    // of the kill plane's query so the test is about the heal rather than about
    // what the arena did next.
    gate.view(subject)
        .get::<&mut Health>(|health| health.current = 0.0);
    gate.view(subject)
        .set(smash::module::lives::RespawnAt(f32::INFINITY));

    gate.advance(1.0);

    assert_eq!(
        observable(gate.health(subject).current),
        0,
        "regeneration healed a player the kill plane had not finished with"
    );
}

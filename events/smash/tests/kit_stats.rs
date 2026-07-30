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
        lobby::{Lobby, LobbyConfig, Phase},
        player::{Energy, Flying, Health, OnGround, Position},
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
///
/// Read off the **seam call** and not the `Health` component, for the reason
/// `knockback_taken_changes_how_far_a_hit_launches` already gives: health the
/// game regenerated and never sent is a bug a state assertion cannot see. This
/// test read the component in the first version of this file, and that is
/// exactly the hole it left -- `regenerate_health` heals through a `&mut Health`
/// query term, `OnSet` does not fire for those, so nothing queued a packet and
/// the client's hearts never moved while the boss bar (which reads the
/// component) said otherwise.
#[test]
fn regen_changes_how_fast_health_comes_back() {
    let gate = Gate::new(|stats| stats.regen = base().regen * 3.0);

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Ability));
    gate.game.server.take();
    gate.advance(4.0);

    gate.differ(
        "the health four seconds of regeneration told the client about",
        |gate, side| {
            let id = gate.id(side.player);
            gate.game
                .server
                .calls()
                .iter()
                .filter_map(|call| match call {
                    Call::SetHealth(told, health, _) if *told == id => Some(observable(*health)),
                    _ => None,
                })
                .next_back()
        },
    );
}

/// Regeneration sends a packet when the bar moves, not once a tick.
///
/// The obvious fix for "regen never reaches the client" is to heal through
/// `.set(Health)` so the `OnSet` mirror fires, and that is twenty packets a
/// second per player forever for a bar with twenty steps on it. This pins the
/// other choice: push only when the half-heart the client draws actually
/// changes.
///
/// The lower bound matters as much as the upper. A test that only capped the
/// count would pass with regeneration sending nothing at all, which is the
/// exact bug this whole thread is about.
#[test]
fn regeneration_sends_a_packet_per_half_heart_and_not_per_tick() {
    let gate = Gate::new(|stats| stats.regen = 1.0);
    // The *variant* side, which is the one `Gate::new` perturbed. Reading
    // `sides[0]` measures the base kit's 0.25/s instead, and the first version
    // of this test did exactly that -- it passed, on a number that meant
    // something other than what this comment claimed.
    let subject = gate.sides[1].player;
    let id = gate.id(subject);

    // `Environment`, so armour does not apply and the arithmetic is legible:
    // ten health off a twenty-point bar is ten half-hearts to climb back.
    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Environment));
    assert_eq!(
        observable(gate.health(subject).current),
        observable(10.0),
        "the hit did not land for what this test's arithmetic assumes"
    );
    gate.game.server.take();

    // Ten seconds at 1.0/s covers all ten, in 200 ticks.
    let seconds = 10.0;
    gate.advance(seconds);
    assert_eq!(
        observable(gate.health(subject).current),
        observable(20.0),
        "regeneration did not reach full, so the packet count is of a shorter climb"
    );

    let sent = gate
        .game
        .server
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::SetHealth(told, _, _) if *told == id))
        .count();

    assert!(
        sent > 0,
        "regeneration sent nothing, so the client's hearts never moved"
    );
    assert!(
        sent <= 12,
        "regeneration sent {sent} packets to climb ten half-hearts; it should send about one per \
         half-heart, not one per tick over {seconds} seconds at {} Hz",
        1.0 / harness::TICK
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

/// Press the jump key in mid-air, the way a client does.
///
/// Deliberately the production path and not a shortcut past it. ENG-11440
/// routes a double jump through vanilla's flight toggle: `arm_double_jump`
/// grants permission while a player is airborne with jumps left, the client
/// answers by setting the flying flag, and `spend_double_jump` reads that
/// mirror. Setting the two mirrors is exactly what the host does with the
/// packet; nothing here reaches past the systems under test to apply an
/// impulse itself, which would make these gates decoration.
fn double_jump(gate: &Gate, side: &Side) {
    let player = gate.view(side.player);
    player.set(OnGround(false));
    // One tick for `arm_double_jump` to grant permission, as it would before a
    // real client could press anything.
    gate.advance(harness::TICK);
    player.set(Flying(true));
}

/// `jump_power`: the double jump's impulse.
#[test]
fn jump_power_changes_how_high_a_double_jump_goes() {
    let gate = Gate::new(|stats| stats.jump_power = base().jump_power * 2.0);

    gate.each(double_jump);
    gate.advance(0.1);

    gate.differ("the impulse the double jump asked for", |gate, side| {
        observable_vec(gate.game.server.total_velocity(gate.id(side.player)))
    });
}

/// `jump_control`: whether the double jump goes where you look.
#[test]
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

/// `jump_count`: how many mid-air jumps a kit gets before touching ground.
///
/// Arrived with ENG-11440 as a tenth `KitStats` field, and this file refused to
/// compile until it was named -- `pattern does not mention field jump_count`,
/// which is the destructure in
/// [`every_kit_stats_field_is_gated_by_a_test_in_this_file`] doing its job on
/// its first real test.
#[test]
fn jump_count_changes_how_many_double_jumps_a_kit_gets() {
    let gate = Gate::new(|stats| stats.jump_count = base().jump_count + 3);

    // More presses than the base kit has jumps, so the two sides diverge on
    // the kit's allowance rather than on how many times the test pressed.
    for _ in 0..4 {
        gate.each(double_jump);
        gate.advance(0.1);
    }

    gate.differ(
        "the total impulse a run of mid-air jumps asked for",
        |gate, side| observable_vec(gate.game.server.total_velocity(gate.id(side.player))),
    );
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
    // One list, two uses. The macro expands to *both* the exhaustive
    // destructure and the name slice, so the two cannot disagree: adding a
    // tenth field breaks compilation at the destructure, and the only edit
    // that fixes it -- naming the field here -- also puts it in the slice and
    // therefore demands a test.
    //
    // Two hand-written lists is what this replaced, and that version was a lie.
    // Compilation broke at the destructure only, so a contributor could add
    // `foo: _` and be green with `foo` ungated -- the same hand-maintained
    // second copy as #1095's array length. The `jump_count` edit had to touch
    // both lists for exactly that reason.
    macro_rules! kit_stats_fields {
        ($($field:ident),+ $(,)?) => {{
            let KitStats { $($field: _),+ } = KitStats::default();
            &[$(stringify!($field)),+]
        }};
    }

    let fields: &[&str] = kit_stats_fields![
        melee_damage,
        armor,
        knockback_taken,
        regen,
        hunger_interval,
        max_health,
        jump_power,
        jump_control,
        jump_count,
        energy,
    ];

    let source = include_str!("kit_stats.rs");
    // `#[test]` functions only. The scan used to collect every `fn`, which put
    // the `double_jump` helper in the list and would have let a helper named
    // `armor_anything` satisfy the `armor` field without a test existing.
    let gated: Vec<&str> = source
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|pair| pair[0] == "#[test]")
        .filter_map(|pair| pair[1].strip_prefix("fn "))
        .filter_map(|line| line.split_once('('))
        .map(|(name, _)| name)
        .collect();

    let missing: Vec<&&str> = fields
        .iter()
        .filter(|field| !gated.iter().any(|name| name.starts_with(*field)))
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

/// The regen scenario reds when `regen` is the field left equal.
///
/// The other half of the deletion check, and it catches a different thing.
/// Deleting `.set(Regen(stats.regen))` proves the field is *read*; this proves
/// the scenario above is sensitive to `regen` specifically and not to something
/// else that happens to differ between two kits. It runs
/// [`regen_changes_how_fast_health_comes_back`] verbatim against a pair whose
/// `regen` is equal and whose `melee_damage` differs instead -- a field that
/// cannot touch a health-after-four-seconds read, because nothing in the
/// scenario swings.
///
/// Without it, a `regen` test whose difference actually came from the armour
/// term would pass with `regen` unimplemented and nobody would know.
#[test]
#[should_panic(expected = "nothing in the game reads that field")]
fn the_regen_gate_reds_when_regen_is_the_field_left_equal() {
    let gate = Gate::new(|stats| stats.melee_damage = base().melee_damage * 2.0);

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Ability));
    gate.advance(4.0);

    gate.differ(
        "the health four seconds of regeneration put back",
        |gate, side| observable(gate.health(side.player).current),
    );
}

/// The hunger scenario reds when `hunger_interval` is the field left equal.
///
/// Same argument as [`the_regen_gate_reds_when_regen_is_the_field_left_equal`],
/// for the other mechanic ENG-11450 implements.
#[test]
#[should_panic(expected = "nothing in the game reads that field")]
fn the_hunger_gate_reds_when_the_interval_is_the_field_left_equal() {
    let gate = Gate::new(|stats| stats.melee_damage = base().melee_damage * 2.0);

    gate.advance(16.0);

    gate.differ("the food bar after sixteen seconds", |gate, side| {
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

/// Every write to the food bar reaches the client.
///
/// The failure mode this is here for is the one #1096 was blocked on, one
/// domain over: game state changes, the seam's cached copy is never told, and
/// the client goes on drawing the old value. `drain_hunger` and
/// `feed_the_attacker` push because they are the paths somebody thought about;
/// the two that *replace* the whole bar -- choosing a kit, and the reset
/// between matches -- are the ones easy to miss, because neither is a hunger
/// mechanic and both look like plain state initialisation.
///
/// Asserted against the seam and not the component. `Hunger::full` obviously
/// sets the component; the question is whether anybody told the player.
#[test]
fn resetting_the_bar_between_matches_tells_the_client() {
    let gate = Gate::new(|stats| stats.hunger_interval = base().hunger_interval / 8.0);

    // Drain both bars well below full, so a refill is a change worth sending.
    gate.advance(8.0);
    let drained = gate.view(gate.sides[1].player).cloned::<&Hunger>().food;
    assert!(drained < smash::module::vitals::FULL, "nothing drained");

    gate.game.server.take();
    // The reset hangs off the transition into `Waiting`, which is where a
    // finished match sends everybody.
    gate.game.world.set(Lobby {
        phase: Phase::Ended,
        timer: 0.0,
    });
    gate.advance(1.0);

    for side in &gate.sides {
        let id = gate.id(side.player);
        assert_eq!(
            gate.view(side.player).cloned::<&Hunger>().food,
            smash::module::vitals::FULL,
            "the reset did not refill the bar"
        );
        assert!(
            gate.game
                .server
                .calls()
                .iter()
                .any(|call| matches!(call, Call::SetFood(told, food)
                    if *told == id && *food == smash::module::vitals::FULL)),
            "the bar was refilled and the client was never told, so it goes on drawing the \
             drained one until the next drain tick"
        );
    }
}

/// Choosing a kit tells the client about the bar it just replaced.
///
/// The other half of the invariant, and honestly labelled: unlike the reset
/// above, this is **not** fixing a divergence that is reachable today.
/// `kit::apply` has one production caller, `lobby::choose`, which refuses a kit
/// change outside `Waiting | Countdown` -- and in those phases the bar is
/// always full, because the clocks only run in `Playing` and the reset refills
/// on the way out. So the push is redundant with the reset right now.
///
/// It is here rather than deleted because "every write that replaces the whole
/// bar tells the client" is the invariant, and an invariant with one exception
/// is a rule nobody can apply. It is tested rather than left as unexercised
/// defence, which is the other way this would have been wrong: the day
/// somebody allows a mid-match kit change, this is already right and this test
/// already covers it.
#[test]
fn choosing_a_kit_tells_the_client_about_the_fresh_bar() {
    let gate = Gate::new(|stats| stats.hunger_interval = base().hunger_interval / 2.0);

    // `Gate::new` applied a kit to all four players as it built the world.
    for side in &gate.sides {
        let id = gate.id(side.player);
        assert!(
            gate.game
                .server
                .calls()
                .iter()
                .any(|call| matches!(call, Call::SetFood(told, food)
                    if *told == id && *food == smash::module::vitals::FULL)),
            "choosing a kit replaced the food bar without telling the client"
        );
    }
}

/// Dying does not refill the food bar.
///
/// A deliberate balance decision and therefore exactly the kind that regresses
/// silently, so it is pinned rather than left in a comment. Hunger is what
/// Super Smash Mobs has instead of sudden death; an anti-stall clock a player
/// can reset by throwing away one of four lives is not a clock. The only two
/// things that refill it are landing a hit and the end of the match.
#[test]
fn dying_does_not_refill_the_food_bar() {
    let gate = Gate::new(|stats| stats.hunger_interval = base().hunger_interval / 8.0);
    let subject = gate.sides[1].player;

    gate.advance(8.0);
    let before = gate.view(subject).cloned::<&Hunger>().food;
    assert!(
        before < smash::module::vitals::FULL,
        "nothing drained, so this would pass with hunger deleted"
    );

    smash::module::lives::kill(gate.view(subject), smash::module::lives::DeathCause::Void);
    // Through the spectate window and back onto a platform.
    gate.advance(smash::module::lives::DEATH_SPECTATE_SECS + 1.0);
    assert!(
        !gate
            .view(subject)
            .has(smash::module::lives::RespawnAt::id()),
        "never respawned, so this says nothing about what a respawn does"
    );

    assert!(
        gate.view(subject).cloned::<&Hunger>().food <= before,
        "dying refilled the food bar, which makes the starve timer optional"
    );
}

/// Neither clock runs outside a match.
///
/// A food bar that drained while a lobby waited for a second player would have
/// somebody starving on the spawn platform before the countdown started.
#[test]
fn the_two_clocks_are_off_in_the_hub() {
    let gate = Gate::new(|stats| stats.regen = base().regen * 3.0);
    // More players required than the four this world has, so `Waiting` is
    // stable for the whole run rather than a countdown that has not fired yet.
    //
    // Setting the phase alone was not enough and the assertion below is how
    // that was found: the harness's default lobby calls four players a full
    // house, so sixteen seconds took this to `Preparing`. The test still
    // passed -- the clocks are gated on `Playing`, which is one phase further
    // on -- so it was quietly asserting something about `Preparing` while
    // claiming to be about the hub, and was one config change from asserting
    // nothing at all.
    gate.game.world.set(LobbyConfig {
        min_players: 8,
        full_players: 16,
        ..LobbyConfig::default()
    });
    gate.game.world.set(Lobby {
        phase: Phase::Waiting,
        timer: 0.0,
    });

    gate.each(|gate, side| gate.hit(side.player, 10.0, DamageKind::Ability));
    let hurt_to = gate.health(gate.sides[0].player).current;
    gate.advance(16.0);
    assert_eq!(
        gate.game.world.cloned::<&Lobby>().phase,
        Phase::Waiting,
        "the lobby left the hub, so this no longer tests what it says"
    );

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

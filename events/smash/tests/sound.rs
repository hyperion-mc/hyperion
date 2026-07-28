//! Every sound the game can make, enumerated from the world rather than listed.
//!
//! A silent ability is the worst kind of bug this game has, because nothing
//! fails: the physics are right, the packets that carry damage and knockback go
//! out, and the only symptom is that the player cannot feel the hit they just
//! landed. There is no assertion a scripted client can make about hearing
//! something, so every check here is on the packet the adapter would encode --
//! the `MockServer` call log, which is the same seam `tests/abilities.rs` reads.
//!
//! The sweeps walk [`ability::manifest`] and [`kit::registry`], which are
//! queries over the world, so a kit added tomorrow is covered the moment its
//! module runs and an ability that quietly declares no sound fails here rather
//! than in play.

mod harness;

use std::collections::BTreeMap;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use hyperion::hyperion_minecraft_proto::generated::registry::SOUND_EVENT;
use smash::{
    module::{
        ability::{self, Declared},
        damage::{DamageKind, Damaged, MatchClock, hurt},
        kit::{self, KitName},
        knockback::Knockback,
        lives::{self, DeathCause, Eliminated, Lives},
        lobby::{Lobby, LobbyConfig, Phase},
        player::{Energy, Health, OnGround, Position},
        sound::{self, Levels, PlaysOnCast, PlaysOnDeath, PlaysOnHurt},
    },
    server::{PlayerId, Sound},
};

/// Whether `id` is a sound event a vanilla client already owns.
///
/// This is the check that makes "use a vanilla sound" enforceable rather than a
/// convention. A sound event the client has never heard of is not an error
/// anywhere: the packet encodes, the client receives it, looks the name up,
/// finds nothing and plays silence. The generated registry is the same table
/// the client resolves against, so a typo fails here instead.
fn is_vanilla(id: &str) -> bool {
    SOUND_EVENT.id_of(id).is_some()
}

/// The whole roster, as a fresh world.
fn manifest() -> Vec<Declared> {
    ability::manifest(&Game::new().world)
}

/// The guard that makes every other sweep in this file mandatory.
///
/// An ability with no sound would pass a test that only checked the sounds that
/// exist, because there would be nothing to check. This is the one that refuses
/// it, and the count is a lower bound rather than an equality so that adding a
/// kit does not require editing a number here -- adding one that forgets a
/// sound still fails.
#[test]
fn every_ability_declares_a_sound() {
    let manifest = manifest();
    assert!(
        manifest.len() >= 50,
        "the registry only found {} abilities; something is not being discovered",
        manifest.len()
    );

    let silent: Vec<_> = manifest
        .iter()
        .filter(|entry| entry.sound.is_empty())
        .map(|entry| format!("{} / {}", entry.kit, entry.name))
        .collect();
    assert!(
        silent.is_empty(),
        "these abilities make no noise when they fire: {silent:?}"
    );
}

/// Two abilities that sound the same are one ability as far as a player's ears
/// are concerned, which defeats the point of giving them sounds at all.
#[test]
fn no_two_abilities_sound_alike() {
    let mut by_sound: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for entry in manifest() {
        by_sound
            .entry(entry.sound)
            .or_default()
            .push(format!("{} / {}", entry.kit, entry.name));
    }
    let shared: Vec<_> = by_sound
        .iter()
        .filter(|(_, users)| users.len() > 1)
        .map(|(sound, users)| format!("{sound} is played by {users:?}"))
        .collect();
    assert!(shared.is_empty(), "{}", shared.join("\n"));
}

/// Every id in the game, from every source, against the client's own table.
///
/// The constants are swept alongside the declarations because they fail exactly
/// the same way and are written in exactly the same place: a string literal
/// nobody has spelled out loud.
#[test]
fn every_sound_id_is_one_a_vanilla_client_owns() {
    let mut wrong = Vec::new();

    for entry in manifest() {
        if !is_vanilla(entry.sound) {
            wrong.push(format!(
                "{} / {} plays {}, which is not a vanilla sound event",
                entry.kit, entry.name, entry.sound
            ));
        }
    }

    let game = Game::new();
    for kit in kit::registry(&game.world) {
        let kit = game.world.entity_from_id(kit);
        let name = kit.try_get::<&KitName>(|n| n.0).unwrap_or("<unnamed>");
        for (occasion, declared) in [
            ("hurt", sound::declared(kit, PlaysOnHurt)),
            ("death", sound::declared(kit, PlaysOnDeath)),
        ] {
            if let Some(declared) = declared
                && !is_vanilla(declared.id)
            {
                wrong.push(format!(
                    "{name}'s {occasion} sound {} is not a vanilla sound event",
                    declared.id
                ));
            }
        }
    }

    for (what, id) in [
        ("impact", sound::IMPACT),
        ("projectile hit", sound::PROJECTILE_HIT),
        ("ranged hitmarker", sound::RANGED_HITMARKER),
        ("countdown tick", sound::COUNTDOWN_TICK),
        ("match start", sound::MATCH_START),
        ("match end", sound::MATCH_END),
        ("elimination", sound::ELIMINATION),
    ] {
        if !is_vanilla(id) {
            wrong.push(format!(
                "the {what} sound {id} is not a vanilla sound event"
            ));
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A kit with no voice is a kit that is silent when it is hit and silent when
/// it dies, which is most of what a player hears in a match.
#[test]
fn every_kit_declares_a_voice() {
    let game = Game::new();
    let kits = kit::registry(&game.world);
    assert!(
        kits.len() >= 15,
        "the registry only found {} kits",
        kits.len()
    );

    let mut mute = Vec::new();
    for kit in kits {
        let kit = game.world.entity_from_id(kit);
        let name = kit.try_get::<&KitName>(|n| n.0).unwrap_or("<unnamed>");
        for (occasion, declared) in [
            ("hurt", sound::declared(kit, PlaysOnHurt)),
            ("death", sound::declared(kit, PlaysOnDeath)),
        ] {
            if declared.is_none() {
                mute.push(format!("{name} declares no {occasion} sound"));
            }
        }
    }
    assert!(mute.is_empty(), "{}", mute.join("\n"));
}

/// The sweep that a declaration alone cannot pass: fire every ability in the
/// game and check the sound it declared actually went out.
///
/// This is the one that catches the relationship being wrong rather than the
/// data. `kit::apply` gives a player their own ability *instances*, made with
/// `is_a` of the kit's prefab, so what a player fires is never the entity a kit
/// declared the sound on. Every check above reads the prefab and would go on
/// passing while every player in the game was silent. Nothing but firing it
/// finds that, and it is what pins the `(OnInstantiate, ...)` choice in
/// `module/sound.rs` whichever way that choice goes.
#[test]
fn firing_an_ability_plays_the_sound_it_declared() {
    let mut failures = Vec::new();

    for entry in manifest() {
        let bench = Bench::new();
        bench.equip(entry.kit);
        if !bench.arm(&entry) {
            failures.push(format!(
                "{} / {}: the Smash Crystal granted nothing, so it could not be fired",
                entry.kit, entry.name
            ));
            continue;
        }
        bench.game.server.take();
        bench.press(&entry);

        let heard: Vec<&str> = bench
            .game
            .server
            .sounds()
            .iter()
            .map(|(_, played)| played.id)
            .collect();
        if !heard.contains(&entry.sound) {
            failures.push(format!(
                "{} / {} declares {} and firing it played {heard:?}",
                entry.kit, entry.name, entry.sound
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// An ultimate is the loudest thing in a match, and volume is range: see
/// `hyperion::net::agnostic::RANGE_PER_VOLUME`. A crystal going off should reach
/// people who are nowhere near it.
#[test]
fn an_ultimate_is_louder_than_an_ordinary_ability() {
    let mut failures = Vec::new();

    for entry in manifest().into_iter().filter(|entry| entry.ultimate) {
        let bench = Bench::new();
        bench.equip(entry.kit);
        assert!(bench.arm(&entry));
        bench.game.server.take();
        bench.press(&entry);

        let volume = bench
            .game
            .server
            .sounds()
            .into_iter()
            .find(|(_, played)| played.id == entry.sound)
            .map(|(_, played)| played.volume);
        match volume {
            Some(volume) if volume >= kit::ULTIMATE_VOLUME => {}
            other => failures.push(format!(
                "{} / {} is an ultimate and played at {other:?}, not {}",
                entry.kit,
                entry.name,
                kit::ULTIMATE_VOLUME
            )),
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A melee swing that connects is heard where it landed.
///
/// Both halves: the impact, which is the same sound for every kit so that its
/// pitch can mean how hard, and the victim's own kit crying out, which is what
/// says who was hit.
#[test]
fn a_melee_connect_is_heard_at_the_victim() {
    let bench = Bench::new();
    bench.equip("Zombie");
    let victim_at = Vec3::new(2.0, 0.0, 0.0);
    bench
        .game
        .world
        .entity_from_id(bench.victim)
        .set(Position(victim_at));
    bench.game.server.take();

    bench.swing(6.0, 1.0);

    let heard = bench.game.server.sounds();
    let impact = heard
        .iter()
        .find(|(_, played)| played.id == sound::IMPACT)
        .unwrap_or_else(|| panic!("a melee hit made no impact sound; the log is {heard:?}"));
    assert!(
        impact.0.distance(victim_at) < 1e-3,
        "the impact played at {} and the victim was at {victim_at}",
        impact.0
    );

    let voice = sound::declared(
        kit::by_name(&bench.game.world, "Zombie").expect("the registry lost Zombie"),
        PlaysOnHurt,
    )
    .expect("Zombie declares a hurt sound");
    assert!(
        heard
            .iter()
            .any(|(at, played)| played.id == voice.id && at.distance(victim_at) < 1e-3),
        "the victim's kit did not cry out; the log is {heard:?}"
    );
}

/// The whole point: a jab and a full smash are the same sound and do not sound
/// the same.
///
/// Driven through the real damage and knockback pipeline rather than by calling
/// `sound::impact` directly, because the claim being made is about what a player
/// hears in a match and the mapping is only half of that. The other half is
/// that the impulse reaching the sound is the one the physics computed.
#[test]
fn a_harder_hit_sounds_different_from_a_light_one() {
    let jab = impact_of(1.0);
    let smash = impact_of(4.0);

    assert!(
        smash.pitch < jab.pitch,
        "a harder hit should be lower: jab {} vs smash {}",
        jab.pitch,
        smash.pitch
    );
    assert!(
        smash.volume > jab.volume,
        "a harder hit should be louder and carry further: jab {} vs smash {}",
        jab.volume,
        smash.volume
    );
    assert_eq!(
        jab.id, smash.id,
        "the two must be the same sound, or the pitch means which kit rather than how hard"
    );
}

/// The impact sound one melee hit with an ability multiplier of `multiplier`
/// produces, taken off the wire rather than computed.
fn impact_of(multiplier: f32) -> Sound {
    let bench = Bench::new();
    bench.equip("Zombie");
    bench.game.server.take();
    bench.swing(6.0, multiplier);
    let heard = bench.game.server.sounds();
    let Some((_, played)) = heard.iter().find(|(_, played)| played.id == sound::IMPACT) else {
        panic!("a hit at x{multiplier} made no impact sound; the log is {heard:?}")
    };
    *played
}

/// The mapping saturates, so the loudest hit in the game is a value a client
/// will honour rather than one it silently clamps.
///
/// A pitch outside `0.5..=2.0` is flattened by the client, which is exactly the
/// moment the sound stops carrying information: every hit past the clamp sounds
/// identical to every other, and the hardest hits are the ones a player most
/// needs to be able to tell apart.
#[test]
fn the_impact_mapping_stays_inside_what_a_client_will_play() {
    for magnitude in [0.05f32, 0.2, 0.7, 1.4, 5.0, 50.0] {
        let played = sound::impact(Vec3::X * magnitude)
            .unwrap_or_else(|| panic!("a hit of {magnitude} blocks a tick was silent"));
        assert!(
            (0.5..=2.0).contains(&played.pitch),
            "a hit of {magnitude} asked for pitch {}, which the client would clamp",
            played.pitch
        );
        assert!(
            played.volume > 0.0,
            "a hit of {magnitude} was played at no volume"
        );
    }

    // Saturating rather than continuing to deepen: two hits well past a full
    // smash are the same sound, which is honest, because they are the same
    // outcome.
    let big = sound::impact(Vec3::X * 5.0).expect("a large hit is audible");
    let bigger = sound::impact(Vec3::X * 50.0).expect("a larger hit is audible");
    assert!((big.pitch - bigger.pitch).abs() < 1e-6);
    assert!((big.volume - bigger.volume).abs() < 1e-6);

    // And a hit that moved nobody says nothing, rather than adding a sound to
    // every zero-knockback ability tick.
    assert!(sound::impact(Vec3::ZERO).is_none());
    assert!(sound::impact(Vec3::X * (sound::QUIET_IMPULSE * 0.5)).is_none());
    assert!(sound::impact(Vec3::X * f32::NAN).is_none());
}

/// Dying plays the kit's last word where it fell; running out of lives adds the
/// elimination on top, because the two look identical on screen.
#[test]
fn dying_and_being_eliminated_sound_different() {
    let bench = Bench::new();
    bench.equip("Creeper");
    let caster = bench.game.world.entity_from_id(bench.caster);
    let at = Vec3::new(5.0, 1.0, -3.0);
    caster.set(Position(at));

    let voice = sound::declared(
        kit::by_name(&bench.game.world, "Creeper").expect("the registry lost Creeper"),
        PlaysOnDeath,
    )
    .expect("Creeper declares a death sound");

    bench.game.server.take();
    lives::kill(caster, DeathCause::Damage);
    let heard = bench.game.server.sounds();
    assert!(
        heard
            .iter()
            .any(|(where_, played)| played.id == voice.id && where_.distance(at) < 1e-3),
        "a death played {heard:?} rather than the kit's death sound at {at}"
    );
    assert!(
        !heard
            .iter()
            .any(|(_, played)| played.id == sound::ELIMINATION),
        "losing one life of four sounded like an elimination"
    );

    // Down to the last one, then take it.
    caster.set(Lives(1));
    caster.set(Position(at));
    bench.game.server.take();
    lives::kill(caster, DeathCause::Damage);
    let heard = bench.game.server.sounds();
    assert!(
        heard
            .iter()
            .any(|(_, played)| played.id == sound::ELIMINATION),
        "running out of lives played {heard:?} and no elimination"
    );
}

/// The last seconds before a match are counted out loud, once each, rising, to
/// everyone.
///
/// Per player rather than positioned: a countdown is about the match and not
/// about a place, so it must not be quieter for whoever is standing furthest
/// from the origin.
#[test]
fn the_countdown_ticks_once_a_second_and_then_the_match_starts() {
    let mut game = Game::new();
    // Two players is enough to start, and a countdown just longer than the
    // audible window so the whole run of ticks is inside one phase.
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 6.0,
        countdown_at_three_quarters: 6.0,
        countdown_at_full: 6.0,
        prepare_seconds: 0.5,
        match_timeout_seconds: 600.0,
        results_seconds: 5.0,
    });
    let listener = game.player("listener", Vec3::ZERO);
    game.player("other", Vec3::new(80.0, 0.0, 0.0));
    let listener = game.world.entity_from_id(listener).cloned::<&PlayerId>();

    game.advance(6.0, 120);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Countdown);

    // Every whole second inside the audible window, once each, rising. Built
    // from the same function the game calls rather than from a literal, so this
    // is a claim about the sequence and not a copy of the pitch table.
    let mut wanted: Vec<f32> = (1..=sound::COUNTDOWN_AUDIBLE_SECONDS)
        .map(|second| sound::countdown_tick(f32::from(second)).pitch)
        .collect();
    wanted.reverse();

    let ticks: Vec<f32> = game
        .server
        .sounds_to(listener)
        .into_iter()
        .filter(|played| played.id == sound::COUNTDOWN_TICK)
        .map(|played| played.pitch)
        .collect();
    assert_eq!(
        ticks,
        wanted,
        "the countdown should tick once a second through its last {} and rise as it goes",
        sound::COUNTDOWN_AUDIBLE_SECONDS
    );

    // Through the prepare phase and into the match.
    game.server.take();
    game.advance(1.0, 20);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);
    assert!(
        game.server
            .sounds_to(listener)
            .iter()
            .any(|played| played.id == sound::MATCH_START),
        "nothing marked the start of the match"
    );

    // And the end, which one player leaving brings on.
    game.server.take();
    game.world
        .entity_from_id(game.players()[1])
        .set(Lives(0))
        .add(Eliminated::id());
    game.advance(1.0, 20);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Ended);
    assert!(
        game.server
            .sounds_to(listener)
            .iter()
            .any(|played| played.id == sound::MATCH_END),
        "nothing marked the end of the match"
    );
}

/// A caster, a victim, and the two things a test here wants to do to them.
struct Bench {
    game: Game,
    caster: Entity,
    victim: Entity,
}

/// Far enough that a victim standing on a splash centre is still launched away
/// from it, and close enough that a melee-range ability reaches. The same
/// reasoning as `tests/abilities.rs`.
const REACH: f32 = 3.5;

impl Bench {
    fn new() -> Self {
        let mut game = Game::new();
        let caster = game.player("caster", Vec3::ZERO);
        let victim = game.player("victim", Vec3::new(REACH, 0.0, 0.0));
        game.world.set(MatchClock(1.0));
        Self {
            game,
            caster,
            victim,
        }
    }

    fn caster(&self) -> EntityView<'_> {
        self.game.world.entity_from_id(self.caster)
    }

    /// Put both of them on `name`, standing up and full of everything.
    fn equip(&self, name: &str) {
        let world = &self.game.world;
        let chosen =
            kit::by_name(world, name).unwrap_or_else(|| panic!("the registry lost {name}"));
        for entity in [self.caster, self.victim] {
            let player = world.entity_from_id(entity);
            kit::apply(world, player, chosen);
            player.set(OnGround(true));
            player.get::<&mut Health>(|health| health.current = health.max);
            if let Some(mut energy) = player.try_get::<&Energy>(|e| *e) {
                energy.current = energy.max;
                player.set(energy);
            }
        }
    }

    /// Hand the caster a Smash Crystal if this entry needs one. `false` only if
    /// one was needed and did not arrive.
    fn arm(&self, entry: &Declared) -> bool {
        if !entry.ultimate || ability::granted_in_slot(self.caster(), entry.slot).is_some() {
            return true;
        }
        kit::grant_ultimate(&self.game.world, self.caster(), 600.0)
    }

    /// Fire `entry` the way a player would, and let anything it launched land.
    fn press(&self, entry: &Declared) {
        match entry.charge_time {
            Some(seconds) => {
                ability::use_slot(self.caster(), entry.slot);
                self.game.advance(seconds, 8);
                ability::release_slot(self.caster(), entry.slot);
            }
            None => ability::use_slot(self.caster(), entry.slot),
        }
        self.game.advance(1.5, 30);
    }

    /// One melee swing from the caster into the victim.
    fn swing(&self, damage: f32, multiplier: f32) {
        let from = self.caster().cloned::<&Position>().0;
        hurt(self.game.world.entity_from_id(self.victim), Damaged {
            attacker: Some(self.caster),
            amount: damage,
            knockback: Knockback::from(from).times(multiplier),
            kind: DamageKind::Melee,
        });
    }
}

/// Redeclaring an occasion's sound replaces it rather than adding a second one.
///
/// The relationships are `Exclusive` and this is what that buys. Without it the
/// second declaration sits alongside the first, `declared` answers with
/// whichever flecs stored first, and a kit that has been retuned goes on
/// playing the sound it used to have with nothing anywhere reporting it.
#[test]
fn declaring_a_sound_twice_keeps_the_second() {
    let game = Game::new();
    let quiet = sound::intern(
        &game.world,
        "minecraft:block.note_block.hat",
        Levels::default(),
    );
    let loud = sound::intern(
        &game.world,
        "minecraft:block.note_block.bell",
        Levels::default(),
    );

    let subject = game.world.entity().add((PlaysOnCast, quiet));
    subject.add((PlaysOnCast, loud));

    let declared = sound::declared(subject, PlaysOnCast).expect("a redeclared sound still reads");
    assert_eq!(
        declared.id, "minecraft:block.note_block.bell",
        "the first declaration outlived the one that replaced it"
    );
}

/// Almost everything here is a sweep over the registry; the three checks that
/// are not name a kit, and a renamed kit would turn those into a panic in a
/// helper rather than a failure anybody can read.
#[test]
fn the_kits_this_file_names_are_still_in_the_registry() {
    let game = Game::new();
    for name in ["Zombie", "Creeper"] {
        assert!(
            kit::by_name(&game.world, name).is_some(),
            "{name} is gone; the hand-written checks in tests/sound.rs need a new subject"
        );
    }
}

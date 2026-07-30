//! What each module needs, and what each module provides, made checkable.
//!
//! `tests/modularity.rs` proves that a *kit* can be added from outside the
//! crate. This is the other half of the same claim, one level down: that the
//! subsystems themselves are separable, so `SmashModule`'s import list is a
//! list of choices rather than an order somebody found by trial and error.
//!
//! The failure it exists to catch is silent. The workspace builds flecs with
//! `flecs_manual_registration`, so a module that reaches for a component
//! another module owns does not get a lazily registered one -- the process
//! aborts. In a match that is a crash. In a normal test it never happens,
//! because every test imports the whole game and every component is therefore
//! already there. So a dependency can be added between two modules and nothing
//! anywhere reports it until the day somebody removes a module from the list.
//!
//! Here each module is imported into a world containing only what it declares
//! it needs, and then actually exercised. A missing declaration aborts the
//! process, which nextest reports as that one test failing. `nix run .#test`
//! runs nextest; under plain `cargo test` the abort takes the whole binary
//! down, which is louder but no less correct.

mod harness;

use std::sync::Arc;

use flecs_ecs::prelude::*;
use glam::{IVec3, Vec3};
use smash::{
    module::{
        ability::AbilityModule,
        arena::{Arena, ArenaModule},
        build_stamp::{BuildStampModule, StampShown},
        damage::{Armor, DamageKind, DamageModule, Damaged, hurt},
        effect::{self, EffectModule},
        hud::HudModule,
        jump::JumpModule,
        kit::{self, KitModule, KitStats},
        kits::StockKits,
        knockback::{Knockback, KnockbackModel, KnockbackModule, KnockbackTaken, Smashed},
        lives::{DeathCause, Lives, LivesModule, kill},
        lobby::{Lobby, LobbyModule, Phase},
        player::{
            self, Energy, Health, JumpPressed, JumpsLeft, OnGround, Player, PlayerModule, Position,
        },
        projectile::ProjectileModule,
        scoreboard::ScoreboardModule,
        selector::{self, SelectorModule, TAKEN_BLOCK},
        sound::{self, Levels, PlaysOnCast, PlaysOnHurt, SoundModule},
    },
    server::{PlayerId, ServerHandle, mock::MockServer},
};

/// One module's declared contract.
struct Contract {
    /// The module, as `SmashModule` names it.
    name: &'static str,
    /// Import it, and nothing else.
    import: fn(&World),
    /// Modules that must be imported before this one can be imported at all,
    /// because this module's systems and observers name their components in a
    /// query signature and flecs resolves those at registration.
    ///
    /// Transitive, and necessarily acyclic: `SmashModule` imports in a straight
    /// line with no resolution step, so a cycle here would mean no order works.
    requires: &'static [&'static str],
    /// Further modules whose components this one touches only while ticking,
    /// because its queries are built inside a `run` closure rather than named
    /// in the system signature.
    ///
    /// These *may* point forwards, and one pair does. `Arena` needs `Lives` and
    /// `Lobby` to run its kill plane while `Lives` needs `Arena` to import at
    /// all, so the two are mutually dependent and only the deferred half of the
    /// cycle makes a linear import list possible. That is worth knowing and is
    /// why it is a separate field rather than folded into `requires`.
    runtime_requires: &'static [&'static str],
    /// Do the thing this module is for, in a world holding the closure of both
    /// requirement sets. Panicking here, or aborting inside flecs, is the
    /// failure.
    exercise: fn(&World, EntityView<'_>),
}

/// The seam the host installs, which belongs to no module.
///
/// `SmashModule` registers these itself before importing anything, so they are
/// the floor every contract below is stated against rather than a dependency
/// any single module could declare.
fn core(world: &World) {
    world.component::<ServerHandle>();
    world.component::<PlayerId>();
    world.set(ServerHandle(Arc::new(MockServer::new())));
}

/// The contracts, in the order `SmashModule` imports them.
///
/// Written by hand. That is the point: a contract derived from the code cannot
/// be violated by the code, and would pass for any import list including a
/// wrong one.
fn contracts() -> Vec<Contract> {
    vec![
        Contract {
            name: "Player",
            import: |world| {
                world.import::<PlayerModule>();
            },
            requires: &[],
            runtime_requires: &[],
            exercise: |world, player| {
                // Energy regenerates. The double jump is `Jump`'s, below: this
                // module owns the components it is played on and none of the
                // rules.
                player.set(Energy::full(10.0, 4.0));
                player.get::<&mut Energy>(|energy| energy.current = 0.0);
                world.progress_time(0.5);
                assert!(
                    player.cloned::<&Energy>().current > 0.0,
                    "energy did not regenerate"
                );
            },
        },
        Contract {
            name: "Knockback",
            import: |world| {
                world.import::<KnockbackModule>();
            },
            requires: &["Player"],
            runtime_requires: &[],
            exercise: |world, player| {
                // The observer reads Health, KnockbackTaken, Position, OnGround
                // and PlayerId, and writes through the server handle. If any of
                // those is not registered, this is where flecs aborts.
                player::notify(player, &Smashed {
                    attacker: None,
                    knockback: Knockback::from(Vec3::ZERO),
                    damage: 6.0,
                });
                assert!(world.cloned::<&KnockbackModel>().min_damage > 0.0);
            },
        },
        Contract {
            name: "Damage",
            import: |world| {
                world.import::<DamageModule>();
            },
            // Damage emits `Smashed`, which is Knockback's event, so it cannot
            // stand up without it. Naming that here is the contract.
            requires: &["Player", "Knockback"],
            // `apply_damage` asks `lives::is_invulnerable` before doing
            // anything, so the damage pipeline reads a component `Lives` owns
            // while `Lives` in turn cannot import without `Damage`'s
            // `MatchClock`. A second mutual pair, found by this test rather
            // than by reading: nothing else in the suite imports one without
            // the other, so nothing else could have noticed.
            //
            // The victim's kit cries out when it is hurt, which walks Sound's
            // `(PlaysOnHurt, sound)` edge from the kit Kit's `Playing` names.
            runtime_requires: &["Lives", "Sound", "Kit"],
            exercise: |_world, player| {
                player.set(Armor(10.0));
                hurt(player, Damaged {
                    attacker: None,
                    amount: 8.0,
                    knockback: Knockback::from(Vec3::ZERO),
                    kind: DamageKind::Melee,
                });
                let health = player.cloned::<&Health>();
                assert!(health.current < health.max, "the hit did not land");
            },
        },
        Contract {
            name: "Sound",
            import: |world| {
                world.import::<SoundModule>();
            },
            // Registration touches only this module's own components. What
            // needs Player is the module's own API: `play_to_everyone` queries
            // `Player` and `PlayerId`, and `position_of` reads `Position`.
            requires: &["Player"],
            // `kit_of` walks the `Playing` relationship, which Kit owns, and
            // Kit cannot import without Ability, which is imported after this.
            // A forward edge, and the reason this field exists.
            runtime_requires: &["Kit"],
            exercise: |world, player| {
                let voice = sound::intern(world, "minecraft:block.note_block.hat", Levels {
                    volume: 0.5,
                    ..Levels::default()
                });
                let subject = world.entity().add((PlaysOnCast, voice));
                let declared = sound::declared(subject, PlaysOnCast)
                    .expect("a sound hung off an entity reads back off it");
                assert!((declared.volume - 0.5).abs() < 1e-6);

                // The two paths that reach out of this module: one through the
                // player's kit, one over every player.
                sound::play_kit_voice(world.into(), player, PlaysOnHurt, Vec3::ZERO);
                sound::play_to_everyone(world.into(), declared);
            },
        },
        Contract {
            name: "Ability",
            import: |world| {
                world.import::<AbilityModule>();
            },
            requires: &["Player", "Knockback", "Damage"],
            // `activate` clears the respawn immunity, which is Lives'; and it
            // reads the firing ability's `(PlaysOnCast, sound)` edge, which is
            // Sound's.
            runtime_requires: &["Lives", "Sound"],
            exercise: |world, player| {
                // No kit, so no ability is granted and this is the empty path
                // through the dispatcher. That it runs at all is the claim.
                smash::module::ability::use_slot(player, 0);
                world.progress_time(0.05);
            },
        },
        Contract {
            name: "Effect",
            import: |world| {
                world.import::<EffectModule>();
            },
            // A tick is a `Damaged` event, so the whole damage chain has to be
            // standing before this module's system can name its terms.
            requires: &["Player", "Knockback", "Damage"],
            // What emitting that event reaches, which is whatever `Damage`
            // itself reaches. Deliberately *not* `Lobby`: durations here are
            // counted in delta time rather than against the match clock, so an
            // effect ticks the same in the hub as in a match. That is the whole
            // reason `Expires` holds a remaining time and not a deadline.
            //
            // Worth knowing about this entry specifically: the closure of these
            // two sets already contains every module in the game, so removing a
            // name from `requires` does not make this test fail. It documents
            // the edge rather than enforcing it. What it does enforce is the
            // exercise below, which fails if either path through the module
            // stops working.
            runtime_requires: &["Lives", "Sound", "Kit"],
            exercise: |world, player| {
                let before = player.cloned::<&Health>().current;
                // No `Cast` and no ability here -- the point is that the module
                // stands up alone -- so the player is blamed for their own burn.
                effect::afflict(
                    world.into(),
                    player,
                    effect::Blame {
                        source: player.id(),
                        attacker: player.id(),
                    },
                    effect::Affliction::over_time(
                        1.0,
                        2.0,
                        0.1,
                        DamageKind::Environment,
                        effect::Shows {
                            effect: smash::module::visuals::burn,
                            sound: "minecraft:entity.player.hurt_on_fire",
                        },
                    ),
                );
                world.progress_time(0.2);
                assert!(
                    player.cloned::<&Health>().current < before,
                    "the effect never ticked"
                );

                // And it ends on its own, taking its entity with it.
                world.progress_time(2.0);
                assert!(
                    effect::on(world.into(), player.id()).is_empty(),
                    "the effect outlived its duration"
                );

                // The other path through the module: a shield adds and later
                // removes `Damage`'s `Immune` tag. Exercised here rather than
                // left to `tests/abilities.rs` because the window ending is the
                // half that silently does not happen, and a shield that never
                // lifts looks exactly like a shield that works.
                effect::afflict(
                    world.into(),
                    player,
                    effect::Blame {
                        source: player.id(),
                        attacker: player.id(),
                    },
                    effect::Affliction::shield(1.0),
                );
                let shielded = player.cloned::<&Health>().current;
                hurt(player, Damaged {
                    attacker: None,
                    amount: 5.0,
                    knockback: Knockback::from(Vec3::ZERO),
                    kind: DamageKind::Ability,
                });
                assert!(
                    (player.cloned::<&Health>().current - shielded).abs() < 1e-6,
                    "the shield did not refuse the hit"
                );

                world.progress_time(1.5);
                hurt(player, Damaged {
                    attacker: None,
                    amount: 5.0,
                    knockback: Knockback::from(Vec3::ZERO),
                    kind: DamageKind::Ability,
                });
                assert!(
                    player.cloned::<&Health>().current < shielded,
                    "the shield outlived its window"
                );
            },
        },
        Contract {
            name: "Kit",
            import: |world| {
                world.import::<KitModule>();
            },
            requires: &["Player", "Knockback", "Damage", "Ability"],
            // Declaring a kit interns its ability and voice sounds.
            runtime_requires: &["Sound"],
            exercise: |world, player| {
                let kit =
                    smash::module::kit::define(world, "Contract", smash::module::kit::KitStats {
                        knockback_taken: 1.5,
                        ..smash::module::kit::KitStats::default()
                    });
                let kit = kit.prefab();
                smash::module::kit::apply(world, player, kit);
                assert!((player.cloned::<&KnockbackTaken>().0 - 1.5).abs() < 1e-6);
            },
        },
        Contract {
            name: "Arena",
            import: |world| {
                world.import::<ArenaModule>();
            },
            // The kill plane's query is built inside a `run` closure, so
            // importing Arena needs nothing of Lives or Lobby. Ticking it needs
            // both, which is the deferred half of the Arena/Lives cycle.
            requires: &["Player", "Knockback", "Damage"],
            runtime_requires: &["Lives", "Lobby"],
            exercise: |world, player| {
                world.set(Lobby {
                    phase: Phase::Playing,
                    timer: 1.0,
                });
                let kill_y = world.cloned::<&Arena>().kill_y;
                player.set(Position(Vec3::new(0.0, kill_y - 10.0, 0.0)));
                world.progress_time(0.05);
                assert!(
                    player.cloned::<&Lives>().0 < smash::module::lives::MAX_LIVES,
                    "the kill plane did not fire"
                );
            },
        },
        Contract {
            name: "Lives",
            import: |world| {
                world.import::<LivesModule>();
            },
            // `smash::respawn` names `&Arena` in its signature, so Arena must
            // be imported first. This is the hard half of the cycle.
            requires: &["Player", "Knockback", "Damage", "Ability", "Kit", "Arena"],
            // A death plays the kit's last word and, on the last life, the
            // elimination.
            runtime_requires: &["Lobby", "Sound"],
            exercise: |_world, player| {
                kill(player, DeathCause::Void);
                assert_eq!(
                    player.cloned::<&Lives>().0,
                    smash::module::lives::MAX_LIVES - 1
                );
            },
        },
        Contract {
            name: "Jump",
            import: |world| {
                world.import::<JumpModule>();
            },
            // The counter and the mirrored press are `Player`'s, the jump
            // power and the per-kit count are `Kit`'s, and the two components
            // that say a player is spectating rather than playing are
            // `Lives`'.
            requires: &["Player", "Kit", "Lives"],
            runtime_requires: &[],
            exercise: |world, player| {
                // A jump is spent and a jump is restored, in that order. Only
                // the restore used to be checked here, and a restore passes
                // with the whole mechanic absent -- which is exactly what it
                // did until ENG-11440.
                player.set(OnGround(true));
                world.progress_time(0.05);
                let allowance = player.cloned::<&JumpsLeft>().0;
                assert!(allowance > 0, "landing did not hand back a jump");

                player.set(OnGround(false));
                player.set(JumpPressed(true));
                world.progress_time(0.05);
                assert_eq!(
                    player.cloned::<&JumpsLeft>().0,
                    allowance - 1,
                    "a mid-air press spent no jump"
                );

                player.set(OnGround(true));
                world.progress_time(0.05);
                assert_eq!(
                    player.cloned::<&JumpsLeft>().0,
                    allowance,
                    "landing did not put the jump back"
                );
            },
        },
        Contract {
            name: "Projectile",
            import: |world| {
                world.import::<ProjectileModule>();
            },
            requires: &["Player", "Knockback", "Damage"],
            runtime_requires: &[],
            exercise: |world, _player| {
                world.progress_time(0.05);
            },
        },
        Contract {
            name: "Lobby",
            import: |world| {
                world.import::<LobbyModule>();
            },
            requires: &[
                "Player",
                "Knockback",
                "Damage",
                "Ability",
                "Kit",
                "Arena",
                "Lives",
            ],
            // The countdown and the two match boundaries.
            runtime_requires: &["Sound"],
            exercise: |world, _player| {
                world.progress_time(0.05);
                assert!(world.cloned::<&Lobby>().timer >= 0.0);
            },
        },
        Contract {
            name: "Scoreboard",
            import: |world| {
                world.import::<ScoreboardModule>();
            },
            requires: &[
                "Player",
                "Knockback",
                "Damage",
                "Ability",
                "Kit",
                "Arena",
                "Lives",
                "Lobby",
            ],
            runtime_requires: &[],
            exercise: |world, _player| {
                world.progress_time(0.05);
            },
        },
        Contract {
            name: "Hud",
            import: |world| {
                world.import::<HudModule>();
            },
            // Everything its one system names in a query term or reads as a
            // singleton: the player's health and held slot, the ability in that
            // slot, the lobby's phase and config, and the knockback model the
            // percentage is derived from.
            requires: &[
                "Player",
                "Knockback",
                "Damage",
                "Ability",
                "Kit",
                "Arena",
                "Lives",
                "Lobby",
            ],
            runtime_requires: &[],
            exercise: |world, player| {
                // A held slot with an ability in it, so the tick walks the
                // whole path rather than the empty one: the mirror is the
                // host's and does not exist here, so the slot is set directly.
                kit::define(world, "HudContract", KitStats::default())
                    .ability(smash::module::kit::AbilitySpec {
                        name: "HudContract",
                        cooldown: 5.0,
                        proves: &[smash::module::ability::Observable::HurtsTarget],
                        ..smash::module::kit::AbilitySpec::DEFAULT
                    })
                    .register();
                let chosen = kit::by_name(world, "HudContract").expect("just defined");
                kit::apply(world, player, chosen);
                // The kit's only ability, so the first slot along.
                player.set(smash::module::player::SelectedSlot(0));
                world.progress_time(0.05);
                world.progress_time(0.05);
            },
        },
        Contract {
            name: "Selector",
            import: |world| {
                world.import::<SelectorModule>();
            },
            // Only `Player`, and only because every world this file builds puts
            // a player in it. The module itself registers three components of
            // its own, declares no system and names nothing else at
            // registration, so it would import into a bare world.
            requires: &["Player"],
            // Everything a selection touches, which is only reached when
            // somebody clicks: the kit registry it builds the ring from, and
            // the lobby whose phase and rules decide whether the click lands.
            runtime_requires: &[
                "Player",
                "Knockback",
                "Damage",
                "Ability",
                "Kit",
                "Arena",
                "Lives",
                "Lobby",
            ],
            exercise: |world, player| {
                // A kit defined here rather than imported, because `StockKits`
                // comes after this module and the point is that the ring is
                // built from whatever the registry holds.
                kit::define(world, "Contract", KitStats::default())
                    .mob("minecraft:creeper")
                    .register();
                selector::build(world, IVec3::ZERO);

                let (_, plinth, _) = selector::podiums(world)
                    .into_iter()
                    .next()
                    .expect("one kit, one podium");
                assert!(
                    selector::click(world, player, plinth.stand()),
                    "the podium did not answer a click"
                );
                assert_eq!(
                    selector::plinths(world).first().map(|(_, block)| *block),
                    Some(TAKEN_BLOCK),
                    "the podium did not turn"
                );
            },
        },
        Contract {
            name: "StockKits",
            import: |world| {
                world.import::<StockKits>();
            },
            // An import-time requirement rather than a runtime one, and the
            // difference is the whole shape of this module: every kit is built
            // inside `module()`, so all fifteen intern their sounds before this
            // import returns.
            requires: &["Player", "Knockback", "Damage", "Sound", "Ability", "Kit"],
            runtime_requires: &[],
            exercise: |world, _player| {
                assert!(
                    smash::module::kit::registry(world).len() >= 10,
                    "the stock kits did not register"
                );
            },
        },
        Contract {
            name: "BuildStamp",
            import: |world| {
                world.import::<BuildStampModule>();
            },
            // Its one system matches `Player` and reads `PlayerId`, which is
            // the seam's. Everything else it touches -- `BuildStamp` itself and
            // the `StampShown` tag -- it registers, in its own registration
            // module, which it imports.
            requires: &["Player"],
            runtime_requires: &[],
            exercise: |world, player| {
                world.progress_time(0.05);
                assert!(
                    player.has(StampShown::id()),
                    "the stamp was never put on the player's screen"
                );
            },
        },
    ]
}

/// Which modules `name` needs standing, in the order to import them.
///
/// Membership and order are computed separately, and that split is the point.
/// Membership is the transitive closure over whichever edge sets `runtime`
/// selects, and the runtime edges genuinely contain a cycle, so no traversal
/// order over them exists. Order is simply the declaration order, which is
/// `SmashModule`'s order, and which
/// [`import_time_requirements_never_point_forwards`] separately proves is a
/// valid one.
///
/// Deriving the order from a traversal instead would be deriving it from the
/// same edges the test is checking, and would happily produce an order that
/// works for a module list nobody ships.
fn closure(all: &[Contract], name: &str, runtime: bool) -> Vec<usize> {
    let index_of = |wanted: &str| {
        all.iter()
            .position(|contract| contract.name == wanted)
            .unwrap_or_else(|| panic!("no module named {wanted}"))
    };

    let mut wanted: Vec<usize> = Vec::new();
    let mut pending = vec![index_of(name)];
    while let Some(next) = pending.pop() {
        if wanted.contains(&next) {
            continue;
        }
        wanted.push(next);
        pending.extend(all[next].requires.iter().map(|r| index_of(r)));
        if runtime {
            pending.extend(all[next].runtime_requires.iter().map(|r| index_of(r)));
        }
    }

    wanted.sort_unstable();
    wanted
}

/// Build a world holding exactly `name`'s closure, and one player in it.
fn world_for(all: &[Contract], name: &str, runtime: bool) -> (World, Entity) {
    let world = World::new();
    core(&world);
    for index in closure(all, name, runtime) {
        (all[index].import)(&world);
    }
    let player = world
        .entity_named("subject")
        .set(PlayerId(1))
        .add(Player::id())
        .id();
    (world, player)
}

/// Run `body` for every module, collecting the ones that fail.
///
/// flecs reports a missing component by panicking, so the panic is caught here
/// and turned into a line naming the module. Without that the failure is one
/// `ECS_INVALID_OPERATION` with no indication of which of eleven modules
/// produced it, which is a report nobody can act on.
fn for_every_module(what: &str, mut body: impl FnMut(&[Contract], &Contract)) {
    let all = contracts();
    let previous = std::panic::take_hook();
    // The default hook prints a backtrace per module, which buries the summary.
    std::panic::set_hook(Box::new(|_| {}));
    let broken: Vec<String> = all
        .iter()
        .filter_map(|contract| {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                body(&all, contract);
            }));
            outcome.err().map(|payload| {
                let detail = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "panicked".to_owned());
                format!("  {}: {detail}", contract.name)
            })
        })
        .collect();
    std::panic::set_hook(previous);

    assert!(
        broken.is_empty(),
        "{what}, but these did not:\n{}\n\nEach line is a module reaching for something no module \
         it declares provides. Either the declaration in this file is out of date, or the module \
         has grown a dependency it should not have.",
        broken.join("\n")
    );
}

/// Every module *imports* with only its declared import-time requirements.
///
/// Registration is where flecs resolves the components a system signature
/// names, so a wrong declaration fails here immediately.
#[test]
fn every_module_imports_with_only_its_declared_requirements() {
    for_every_module(
        "every module imports with only what it declares",
        |all, contract| {
            drop(world_for(all, contract.name, false));
        },
    );
}

/// Every module *runs* with only its declared requirements, import-time and
/// deferred.
///
/// The stronger claim, and the one that catches a module reaching into another
/// module's components from inside a `run` closure -- which registration cannot
/// see and which no test importing the whole game will ever notice.
#[test]
fn every_module_runs_with_only_its_declared_requirements() {
    for_every_module(
        "every module runs with only what it declares",
        |all, contract| {
            let (world, player) = world_for(all, contract.name, true);
            let player = world.entity_from_id(player);
            // Position and ground state belong to Player, so a module that requires
            // Player gets them and one that does not never sees them.
            if player.try_get::<&Position>(|_| ()).is_some() {
                player.set(Position(Vec3::new(0.0, 40.0, 0.0)));
                player.set(OnGround(true));
            }
            (contract.exercise)(&world, player);
        },
    );
}

/// Import-time requirements only ever point backwards.
///
/// `SmashModule` imports in a straight line with no dependency resolution, so
/// a forward edge here would mean no single order satisfies everything and the
/// list only works by accident. Deferred edges are exempt: one genuinely points
/// forward, and `Contract::runtime_requires` says which and why.
#[test]
fn import_time_requirements_never_point_forwards() {
    let all = contracts();
    for (index, contract) in all.iter().enumerate() {
        for required in contract.requires {
            let at = all
                .iter()
                .position(|other| other.name == *required)
                .unwrap_or_else(|| panic!("{} requires unknown module {required}", contract.name));
            assert!(
                at < index,
                "{} needs {required} at import time, but SmashModule imports {required} after it",
                contract.name
            );
        }
    }
}

/// The modules `SmashModule` imports, in order, read out of the source.
///
/// Scoped to `SmashModule`'s own block: `SmashHost` in the same file imports
/// hyperion's modules too, and those are the host's business rather than the
/// game's.
fn smash_module_imports() -> Vec<&'static str> {
    let source = include_str!("../src/lib.rs");
    let start = source
        .find("impl Module for SmashModule")
        .expect("SmashModule is defined in lib.rs");
    let rest = &source[start..];
    let end = rest[1..]
        .find("\nimpl ")
        .map_or(rest.len(), |offset| offset + 1);

    rest[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("world.import::<"))
        .filter_map(|line| line.split_once(">()"))
        .map(|(name, _)| name)
        .collect()
}

/// The contract list and the game's own import list name the same modules.
///
/// Without this the file rots: a module added to `SmashModule` and not here is
/// simply never contract tested, and the suite stays green while the thing it
/// claims to check quietly stops being true.
#[test]
fn the_contract_list_covers_every_module_the_game_imports() {
    let imported = smash_module_imports();

    let declared: Vec<String> = contracts()
        .iter()
        .map(|contract| {
            if contract.name == "StockKits" {
                contract.name.to_owned()
            } else {
                format!("{}Module", contract.name)
            }
        })
        .collect();

    for name in &imported {
        assert!(
            declared.iter().any(|held| held == name),
            "SmashModule imports {name}, which has no contract in this file"
        );
    }
    assert_eq!(
        imported.len(),
        declared.len(),
        "the contract list and SmashModule's import list disagree: {imported:?} against \
         {declared:?}"
    );
}

/// The declared order is the order the game actually imports in.
///
/// The contract list claims to be `SmashModule`'s list; if the two drift, every
/// statement above about what comes before what is about a different program.
#[test]
fn the_contract_list_is_in_the_games_import_order() {
    let imported = smash_module_imports();

    for (declared, actual) in contracts().iter().zip(&imported) {
        let expected = actual.strip_suffix("Module").unwrap_or(actual);
        assert_eq!(
            declared.name, expected,
            "the contract list is out of order against SmashModule"
        );
    }
}

/// Importing a module twice is harmless.
///
/// flecs makes `import` idempotent and the game relies on it: several modules
/// name the same requirement, so resolving a closure naively imports `Player`
/// half a dozen times.
#[test]
fn importing_a_module_twice_is_harmless() {
    let all = contracts();
    let world = World::new();
    core(&world);
    for contract in &all {
        (contract.import)(&world);
    }
    for contract in &all {
        (contract.import)(&world);
    }
    let player = world
        .entity_named("subject")
        .set(PlayerId(1))
        .add(Player::id())
        .set(Position(Vec3::new(0.0, 40.0, 0.0)))
        .set(OnGround(true));
    world.progress_time(0.05);
    assert_eq!(player.cloned::<&Lives>().0, smash::module::lives::MAX_LIVES);
}

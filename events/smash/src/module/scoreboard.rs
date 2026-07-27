//! The sidebar, and the spectator flag that goes with being out.
//!
//! Mineplex's scoreboard is one line per player with their remaining lives as
//! the score, collapsing to two aggregate lines above fourteen players. Both
//! shapes are here because the collapse is what makes the mode playable in a
//! big lobby, and the colour of a name is the only warning you get that
//! somebody is on their last life.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        lives::{Eliminated, Lives},
        lobby::{Lobby, Phase},
        player::Player,
    },
    server::{PlayerId, ServerHandle},
};

/// Above this many players the per-player list is replaced by two counters.
pub const COLLAPSE_ABOVE: usize = 14;

/// One player's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub lives: u8,
    pub colour: &'static str,
}

/// Build the sidebar body. Pure, so the layout is testable without a server.
#[must_use]
pub fn render(phase: Phase, mut rows: Vec<Row>) -> Vec<String> {
    rows.sort_by(|a, b| b.lives.cmp(&a.lives).then_with(|| a.name.cmp(&b.name)));

    if rows.len() > COLLAPSE_ABOVE {
        let alive = rows.iter().filter(|row| row.lives > 0).count();
        return vec![
            format!("Players Alive: {alive}"),
            format!("Players Dead: {}", rows.len() - alive),
        ];
    }

    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| format!("[{}] {} {}", row.colour, row.name, row.lives))
        .collect();

    if matches!(phase, Phase::Waiting | Phase::Countdown) {
        lines.push("Waiting for players".to_owned());
    }
    lines
}

/// The sidebar as it was last sent, so an unchanged one is not sent again.
///
/// A redraw is four packets plus one per row, per viewer. Doing that every tick
/// is twenty times a second of bandwidth spent restating a line that changes
/// when somebody dies, which is the difference between a few hundred packets a
/// minute and a few hundred thousand.
#[derive(Component, Debug, Default)]
struct Drawn {
    lines: Vec<String>,
    viewers: Vec<PlayerId>,
}

#[derive(Component)]
pub struct ScoreboardModule;

impl Module for ScoreboardModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Scoreboard");

        world.component::<Drawn>().add_trait::<flecs::Singleton>();
        world.set(Drawn::default());

        world
            .system_named::<()>("smash::update_scoreboard")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    let phase = world.cloned::<&Lobby>().phase;

                    let mut rows = Vec::new();
                    let mut viewers = Vec::new();
                    world
                        .query::<(&Lives, &PlayerId)>()
                        .with(Player::id())
                        .build()
                        .each_entity(|player, (lives, id)| {
                            let lives = if player.has(Eliminated::id()) {
                                Lives(0)
                            } else {
                                *lives
                            };
                            rows.push(Row {
                                name: player.name(),
                                lives: lives.0,
                                colour: lives.colour(),
                            });
                            viewers.push(*id);
                        });

                    let lines = render(phase, rows);
                    let unchanged = world
                        .get::<&Drawn>(|drawn| drawn.lines == lines && drawn.viewers == viewers);
                    if unchanged {
                        continue;
                    }

                    world.get::<&ServerHandle>(|server| {
                        for viewer in &viewers {
                            server.set_sidebar(*viewer, "Super Smash Mobs", &lines);
                        }
                    });
                    world.set(Drawn { lines, viewers });
                }
            });

        // Elimination is the only thing that turns spectating permanently on;
        // respawning turns it back off. Keeping it as an observer rather than a
        // poll means a player is a spectator on the same tick they lose their
        // last life, not on the next one.
        world
            .observer_named::<crate::module::lives::EliminatedEvent, &PlayerId>(
                "smash::spectate_on_elimination",
            )
            .with(Player::id())
            .each_iter(|it, _index, id| {
                it.world()
                    .get::<&ServerHandle>(|server| server.set_spectating(*id, true));
            });
    }
}

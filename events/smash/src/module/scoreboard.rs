//! The sidebar, and the spectator flag that goes with being out.
//!
//! Mineplex's scoreboard is one line per player with their remaining lives as
//! the score, collapsing to two aggregate lines above fourteen players. Both
//! shapes are here because the collapse is what makes the mode playable in a
//! big lobby, and the colour of a name is the only warning you get that
//! somebody is on their last life.
//!
//! # The row is a component, and the number is the score
//!
//! A row used to be a `String` built with `format!("[{}] {} {}", colour, name,
//! lives)`, and the sidebar had three separate faults on screen at once.
//!
//! The colour was a placeholder `&'static str` nothing consumed, so `[green]`
//! reached a real client as literal text and the last-life warning this module
//! is built around had never once fired.
//!
//! The number in the red column was not the lives. The adapter set every row's
//! score to `lines.len() - index`, a reverse row index, so a panel of two
//! players and a status line read 3, 2, 1 down the side whatever anybody's
//! lives were. That is a sort key the protocol requires, drawn to the player
//! because nothing told the client to hide it, and read by the player as data.
//! Two rows both reading `Emerald_Explorer 4` against scores of 2 and 1 is
//! that bug: the lives agree, the numbers beside them do not, and neither
//! number is a life.
//!
//! Both faults widened the rows. Minecraft draws the score hard against the
//! right edge and sizes the panel to its widest row, so a row carrying markup
//! and a life count it did not need pushed the panel out until the red number
//! sat on top of the name.
//!
//! All three are the same mistake, which is describing presentation in a
//! string instead of in the structure the protocol already has. A row is now a
//! [`Text`] with a real colour and a [`Score`] beside it, and
//! [`Server::set_sidebar`](crate::server::Server::set_sidebar) will not accept
//! anything else.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        lives::{self, Lives},
        lobby::{Lobby, Phase},
        player::Player,
    },
    server::{NamedColor, PlayerId, Score, ServerHandle, SidebarLine, Text},
};

/// Above this many players the per-player list is replaced by two counters.
pub const COLLAPSE_ABOVE: usize = 14;

/// The sidebar's title.
pub const TITLE: &str = "Super Smash Mobs";

/// How many characters wide a row may be, score included.
///
/// The client draws the row text from the left of the panel and the score hard
/// against the right, and sizes the panel to the widest row it holds. Nothing
/// clips a row on its own; what happens instead is that one long row widens
/// the whole panel until it reaches the parts of the screen a player needs,
/// and the red score is what lands on top of the text. So the budget is a
/// property of the panel and not of any one row, and every row is measured
/// against the same number.
///
/// Thirty is chosen against the content: a Minecraft name is at most sixteen
/// characters and the widest fixed string here is `Waiting for players` at
/// nineteen, so nothing the game generates is truncated, and a row that would
/// have to be is one somebody added without thinking about the panel.
pub const SIDEBAR_WIDTH: usize = 30;

/// What a row that will not fit ends with. Two dots and not an ellipsis
/// character, because the client's font is not guaranteed to have one.
const ELLIPSIS: &str = "..";

/// One player's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub lives: u8,
    /// The colour the name is drawn in, which is the whole warning that
    /// somebody is on their last life. Chosen from the same tier table that
    /// the `(ShownAs, tier)` relation points into, so the sidebar and anything
    /// querying that relation cannot disagree about where the bands are.
    pub colour: NamedColor,
}

/// Characters the score column takes on a row, gutter included.
///
/// Zero for a row whose number is not drawn, because that row's whole width is
/// its own. One space of gutter otherwise, so the text and the number never
/// touch even when a row spends its entire budget.
fn score_width(score: Score) -> usize {
    // `i32::to_string` is the same rendering the client does, minus the font.
    score
        .drawn()
        .map_or(0, |value| value.to_string().chars().count() + 1)
}

/// `text`, shortened until it and `score` together fit [`SIDEBAR_WIDTH`].
fn fit(text: &str, score: Score) -> String {
    let budget = SIDEBAR_WIDTH.saturating_sub(score_width(score));
    if text.chars().count() <= budget {
        return text.to_owned();
    }
    let keep = budget.saturating_sub(ELLIPSIS.chars().count());
    text.chars().take(keep).chain(ELLIPSIS.chars()).collect()
}

/// A row of plain text with no colour of its own.
fn plain_line(text: &str, score: Score) -> SidebarLine {
    SidebarLine {
        text: Text::text(fit(text, score)),
        score,
    }
}

/// Build the sidebar body. Pure, so the layout is testable without a server.
///
/// Lives are the score, and the row text is the name alone.
///
/// The score is a slot the client owns: it right-aligns it, colours it red and
/// reserves width for it whether or not the row has anything to put there. A
/// row therefore pays for that column no matter what, so the only question is
/// what goes in it. Before this it was a reverse row index, which is a number
/// the player cannot use and the game does not mean, while the lives it could
/// have held were spelled out in the text and spending the row's scarce left
/// half. Putting the lives where the number already is says the same thing in
/// a column that was being paid for anyway, and hands the whole budget back to
/// the name.
#[must_use]
pub fn render(phase: Phase, mut rows: Vec<Row>) -> Vec<SidebarLine> {
    rows.sort_by(|a, b| b.lives.cmp(&a.lives).then_with(|| a.name.cmp(&b.name)));

    if rows.len() > COLLAPSE_ABOVE {
        let alive = rows.iter().filter(|row| row.lives > 0).count();
        let dead = rows.len() - alive;
        // The counts stay in the text and the score is rank only. A count is
        // what these two rows are *about*, not a rank, and putting it in the
        // score column would both hide it behind a right-aligned number and
        // reorder the panel whenever the dead outnumbered the living.
        return vec![
            plain_line(&format!("Players Alive: {alive}"), Score::Rank(2)),
            plain_line(&format!("Players Dead: {dead}"), Score::Rank(1)),
        ];
    }

    let mut lines: Vec<SidebarLine> = rows
        .iter()
        .map(|row| {
            let score = Score::Shown(i32::from(row.lives));
            SidebarLine {
                text: Text::text(fit(&row.name, score)).color(row.colour),
                score,
            }
        })
        .collect();

    if matches!(phase, Phase::Waiting | Phase::Countdown) {
        // Rank zero, below every player row: a player out of lives also scores
        // zero, and ties are broken by the row key the adapter assigns in this
        // order, so the status stays at the bottom where it belongs.
        lines.push(plain_line("Waiting for players", Score::Rank(0)));
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
    lines: Vec<SidebarLine>,
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
            .system_named::<()>("update_scoreboard")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    let phase = world.cloned::<&Lobby>().phase;

                    // Once per tick, not once per player, and read from the
                    // tier table rather than off each player's `(ShownAs,
                    // tier)` edge. See `Bands`: that edge is written by a
                    // deferred command, so on the tick a player joins it is
                    // not there yet and this panel would leave them out of
                    // both the rows and the viewers.
                    let bands = lives::Bands::of(&world);

                    let mut rows = Vec::new();
                    let mut viewers = Vec::new();
                    world
                        .query::<(&Lives, &PlayerId)>()
                        .with(Player::id())
                        .build()
                        .each_entity(|player, (lives, id)| {
                            let remaining = lives::remaining(player, *lives);
                            rows.push(Row {
                                name: player.name(),
                                lives: remaining,
                                colour: bands.tint(remaining).expect(
                                    "LivesModule builds the tier table at import and \
                                     ScoreboardModule declares it as a requirement",
                                ),
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
                            server.set_sidebar(*viewer, Text::text(TITLE), &lines);
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
                "spectate_on_elimination",
            )
            .with(Player::id())
            .each_iter(|it, _index, id| {
                it.world()
                    .get::<&ServerHandle>(|server| server.set_spectating(*id, true));
            });
    }
}

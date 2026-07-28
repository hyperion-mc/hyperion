//! Boss bars: the strip across the top of a client's screen.
//!
//! # A bar is an entity and its audience is a set of edges
//!
//! ```ignore
//! let bar = world
//!     .entity()
//!     .add(BossBar::id())
//!     .set(Title(Text::text("CPU 240% of 1600%").color(NamedColor::Red)))
//!     .set(Progress(0.15))
//!     .add((ShownTo, operator));
//! ```
//!
//! and later `bar.set(Progress(0.31))`, which sends one `UpdateProgress` to
//! each viewer and nothing else. There is no packet in a caller anywhere.
//!
//! # Why the sent state is pair data and not a table
//!
//! [`Sent`] holds exactly what one client was last told, as the data on the
//! `(Sent, viewer)` pair. That one choice is what makes the two cases that are
//! usually bugs stop being code at all:
//!
//! * **Somebody joins mid-match.** They have no `(Sent, bar)` pair, so the
//!   next tick sends them `Add` for every bar whose audience they are in.
//!   Joining is the absence of state rather than a handler.
//! * **Somebody leaves, or a bar is destroyed, or the audience shrinks.** All
//!   three end with the pair going away -- flecs removes it itself for the
//!   first two, under the `(OnDeleteTarget, Remove)` cleanup written down in
//!   [`BossBarModule::module`] -- and one `OnRemove` observer on
//!   `(Sent, Wildcard)` sends the `Remove`. Teardown cannot leak, because the
//!   fact that says "this client has this bar" and the thing that gets cleaned
//!   up are the same fact.
//!
//! # Nothing unchanged is resent
//!
//! The drive system compares each viewer's [`Sent`] against the bar's
//! components and emits only the operations whose field moved. An unchanged
//! bar sends nothing at all. That is not a micro-optimisation: hyperion#1018
//! measured 3,065 boss bar packets in thirty seconds for eight clients from a
//! bar that resent itself whenever its progress moved by any amount, against
//! 1,135 once the game quantised what it asked for. A bar is a packet per
//! viewer per change, and a caller that sets the same value every tick is the
//! normal case rather than the exceptional one.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    Uuid,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::player::{
        BossBarColor, BossBarOverlay, BossBarProperties, BossEvent, BossEventOperation,
    },
};
use tracing::warn;

use crate::{
    ingress::PendingRemove,
    net::{Compose, ConnectionId, protocol::Clientbound},
};

/// A boss bar's text.
///
/// A whole component and never a `String`, so a caller cannot smuggle a colour
/// in as markup: writing `"§cCPU"` into a title is what hyperion#1004 existed
/// to stop, and a type that only accepts components is the only version of
/// that rule which cannot be forgotten.
pub type Text = hyperion_minecraft_proto::text::Component<'static>;

/// Tag: this entity is a boss bar.
#[derive(Component, Debug)]
pub struct BossBar;

/// What the bar says, drawn centred above it.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Title(pub Text);

/// How full the bar is, `0.0..=1.0`.
///
/// Anything outside that range, and anything not finite, is clamped on the way
/// to the wire rather than rejected here: a bar is a readout and the fill is
/// the least important half of it, so a caller with a bad number should still
/// get their title on screen.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Progress(pub f32);

/// How the bar is drawn.
///
/// The protocol's own enums rather than integers, so `overlay: 3` cannot be
/// written where `Notched12` was meant and the set of legal values is the set
/// the compiler will accept.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    /// The bar's tint.
    pub colour: BossBarColor,
    /// One continuous bar, or six, ten, twelve or twenty notches.
    pub overlay: BossBarOverlay,
}

/// What the bar does to the world around it: darken the sky, play boss music,
/// close fog in.
///
/// Absent means [`BossBarProperties::NONE`], because every one of these
/// changes how the game looks or sounds and a bar that is a readout should
/// change neither.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Effects(pub BossBarProperties);

/// Relationship: `(ShownTo, player)` on a bar. The audience, as edges.
#[derive(Component, Debug)]
pub struct ShownTo;

/// Tag on a bar whose audience is every client in play.
///
/// Separate from [`ShownTo`] rather than an edge to a group entity, because
/// "everyone" is a rule and not a list: a player who connects a minute from
/// now is in it, and nothing has to be told about them.
#[derive(Component, Debug)]
pub struct ShownToEveryone;

/// Relationship data: `(Sent, viewer)` on a bar, holding exactly what that
/// client was last told.
///
/// Every field is the value that actually went on the wire, after clamping, so
/// comparing against it answers "does this client already have this" rather
/// than "did the caller ask for the same thing".
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Sent {
    title: Text,
    progress: f32,
    style: Style,
    effects: Effects,
}

/// The high 64 bits of every boss bar id hyperion mints.
///
/// A bar is addressed by a UUID the *server* chooses and reuses for the life
/// of the bar, so it has to be stable and it has to be distinct between bars.
/// The bar's own entity id is both, for exactly as long as the bar exists, so
/// it is the low half and this is a namespace keeping these ids away from any
/// a game mints for itself. Derived and not stored, so there is no table that
/// can fall out of step with entity deletion.
const NAMESPACE: u64 = u64::from_be_bytes(*b"hyp_boss");

/// The id `bar` is addressed by on the wire.
#[must_use]
pub const fn bar_uuid(bar: Entity) -> Uuid {
    Uuid(((NAMESPACE as u128) << 64) | bar.0 as u128)
}

impl Sent {
    /// What a bar's components say it should look like.
    fn of(
        title: &Title,
        progress: Progress,
        style: Option<&Style>,
        effects: Option<&Effects>,
    ) -> Self {
        Self {
            title: title.0.clone(),
            progress: fill(progress.0),
            style: style.copied().unwrap_or_default(),
            effects: effects.copied().unwrap_or_default(),
        }
    }

    /// Whether this client's fill is `other`'s.
    ///
    /// Bitwise, because the question is whether the same four bytes would go
    /// on the wire again, which is exactly what a bit pattern answers and what
    /// a float comparison only approximates.
    const fn same_fill(&self, other: &Self) -> bool {
        self.progress.to_bits() == other.progress.to_bits()
    }
}

/// A caller's progress as the wire will carry it.
///
/// Not finite reads as empty rather than as a rejected bar: `f32::clamp`
/// propagates `NaN`, and a `NaN` on the wire is a bar the client draws at an
/// arbitrary width forever.
const fn fill(progress: f32) -> f32 {
    if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn push(compose: &Compose, to: ConnectionId, id: Uuid, operation: BossEventOperation<'_>) {
    let packet = BossEvent { id, operation };
    if let Err(error) = compose.unicast(Clientbound::new(PacketId::BossEvent.to_raw(), &packet), to)
    {
        warn!("dropping a boss bar packet: {error}");
    }
}

/// The connection to draw on, or nothing when there is nobody there any more.
///
/// [`PendingRemove`] and not only liveness: a disconnecting player is destroyed
/// in `PostLoad`, and flecs runs the `(Sent, player)` removal that destruction
/// causes while the entity is still alive and still carrying the
/// [`ConnectionId`] it had. Sending a `Remove` down that is a packet written
/// for a socket that has already gone.
fn viewer_connection(viewer: EntityView<'_>) -> Option<ConnectionId> {
    if !viewer.is_alive() || viewer.has(id::<PendingRemove>()) {
        return None;
    }
    viewer.try_get::<&ConnectionId>(|id| *id)
}

/// Whether `viewer` is still in `bar`'s audience.
fn in_audience(bar: EntityView<'_>, viewer: EntityView<'_>) -> bool {
    if bar.has(id::<ShownToEveryone>()) {
        return viewer.is_alive() && viewer.has(id::<ConnectionId>());
    }
    bar.has((id::<ShownTo>(), viewer.id()))
}

#[derive(Component)]
pub struct BossBarModule;

impl Module for BossBarModule {
    fn module(world: &World) {
        world.component::<BossBar>();
        world.component::<Title>();
        world.component::<Progress>();
        world.component::<Style>();
        world.component::<Effects>();
        world.component::<ShownToEveryone>();

        // Deleting a viewer removes what that viewer was told, and leaves the
        // bar standing for everybody else still looking at it. This is flecs's
        // default and it is spelled out because the teardown observer below is
        // built on it: it is the difference between one player disconnecting
        // and every player losing the bar.
        world
            .component::<ShownTo>()
            .add_trait::<(flecs::OnDeleteTarget, flecs::Remove)>();
        world
            .component::<Sent>()
            .add_trait::<(flecs::OnDeleteTarget, flecs::Remove)>();

        // The whole of teardown. A bar stops being shown in three ways -- the
        // bar is destroyed, the viewer disconnects, or the audience shrinks --
        // and all three end here, because all three end with this pair going
        // away.
        //
        // One term, deliberately. A second filter term turns an observer into
        // a query-match observer, which fires on any table transition that
        // still satisfies the query rather than on the removal itself.
        world
            .observer_named::<flecs::OnRemove, ()>("boss_bar_teardown")
            .with((id::<Sent>(), id::<flecs::Wildcard>()))
            .each_iter(|it, row, ()| {
                let world = it.world();
                let bar = it.entity(row);
                let viewer = world.entity_from_id(it.pair(0).second_id().id());
                let Some(connection) = viewer_connection(viewer) else {
                    return;
                };
                world.get::<&Compose>(|compose| {
                    push(
                        compose,
                        connection,
                        bar_uuid(bar.id()),
                        BossEventOperation::Remove,
                    );
                });
            });

        let bars = world
            .query::<(&Title, &Progress, Option<&Style>, Option<&Effects>)>()
            .with(id::<BossBar>())
            .build();
        let shown = world
            .query::<(
                &Title,
                &Progress,
                Option<&Style>,
                Option<&Effects>,
                &mut (Sent, flecs::Wildcard),
            )>()
            .with(id::<BossBar>())
            .build();
        let everyone = world.new_query::<&ConnectionId>();

        world
            .system_named::<&Compose>("boss_bar_sync")
            .kind(id::<flecs::pipeline::OnStore>())
            .each(move |compose| {
                // First: anybody in an audience who has never been told. That
                // set is the joiners, the bars created this tick, and the
                // audiences that just grew, and none of the three has to be
                // told apart from the others.
                bars.each_entity(|bar, (title, progress, style, effects)| {
                    let state = Sent::of(title, *progress, style, effects);
                    if bar.has(id::<ShownToEveryone>()) {
                        everyone.each_entity(|viewer, _| add_if_new(compose, bar, viewer, &state));
                    } else {
                        bar.each_target(id::<ShownTo>(), |viewer| {
                            add_if_new(compose, bar, viewer, &state);
                        });
                    }
                });

                // Then: everybody who has been told something, compared
                // against what the bar says now.
                shown.each_iter(|it, row, (title, progress, style, effects, sent)| {
                    let world = it.world();
                    let bar = it.entity(row);
                    let viewer = world.entity_from_id(it.pair(4).second_id().id());
                    if !in_audience(bar, viewer) {
                        // Which fires the observer above, which is what
                        // actually takes the bar off the screen.
                        bar.remove((id::<Sent>(), viewer.id()));
                        return;
                    }
                    let Some(connection) = viewer_connection(viewer) else {
                        return;
                    };
                    let now = Sent::of(title, *progress, style, effects);
                    let Some(operation) = operation(sent, &now) else {
                        return;
                    };
                    push(compose, connection, bar_uuid(bar.id()), operation);
                    *sent = now;
                });
            });
    }
}

/// Send `Add` to a viewer who has never seen this bar, and record it.
///
/// A viewer with no connection yet is skipped *without* recording anything, so
/// the bar arrives on the tick they gain one rather than being counted as
/// already delivered. That is the same rule as a mid-match joiner and it is
/// the same line of code.
fn add_if_new(compose: &Compose, bar: EntityView<'_>, viewer: EntityView<'_>, state: &Sent) {
    if bar.has((id::<Sent>(), viewer.id())) {
        return;
    }
    let Some(connection) = viewer_connection(viewer) else {
        return;
    };
    push(compose, connection, bar_uuid(bar.id()), add(state));
    bar.set_first(state.clone(), viewer.id());
}

/// The whole bar, in one packet.
fn add(state: &Sent) -> BossEventOperation<'_> {
    BossEventOperation::Add {
        name: state.title.clone(),
        progress: state.progress,
        color: state.style.colour,
        overlay: state.style.overlay,
        properties: state.effects.0,
    }
}

/// The one packet that takes a client from `sent` to `now`, if it needs one.
///
/// Exactly one field moved: the operation for that field, carrying nothing
/// else. Nothing moved: no packet, which is the rule that took hyperion#1018's
/// bar from 3,065 packets in thirty seconds to 1,135.
///
/// Several fields moved: a fresh `Add`, and not the run of updates it could be
/// instead. Two reasons, pointing the same way.
///
/// It is **atomic**. Each operation is its own packet, so a run of them leaves
/// the client holding a bar that is half one state and half the next, and
/// there is no ordering that fixes it -- put the title last and a countdown
/// ticking over shows the next second's fill under this second's number; put
/// it first and a lobby bar becoming a countdown is a countdown drawn in the
/// lobby's blue. `nix run .#smash-hud-e2e` caught the second of those as a
/// real failure, which is how this arrived.
///
/// It is **smaller**. Every operation repeats the sixteen byte id, so two
/// updates already cost more than the `Add` that carries all four fields, and
/// the client is built for it: `BossHealthOverlay.add` is a `put` into a map
/// keyed on the id, so a second `Add` replaces the bar in place and keeps its
/// slot on screen.
///
/// What that gives up is the client-side lerp between two progress values,
/// which none of these bars want: a knockback percentage steps when somebody
/// is hit, and sliding it smoothly animates something that did not happen.
fn operation<'a>(sent: &Sent, now: &'a Sent) -> Option<BossEventOperation<'a>> {
    match (
        sent.title != now.title,
        !sent.same_fill(now),
        sent.style != now.style,
        sent.effects != now.effects,
    ) {
        (false, false, false, false) => None,
        (true, false, false, false) => Some(BossEventOperation::UpdateName(now.title.clone())),
        (false, true, false, false) => Some(BossEventOperation::UpdateProgress(now.progress)),
        (false, false, true, false) => Some(BossEventOperation::UpdateStyle {
            color: now.style.colour,
            overlay: now.style.overlay,
        }),
        (false, false, false, true) => Some(BossEventOperation::UpdateProperties(now.effects.0)),
        _ => Some(add(now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(progress: f32) -> Sent {
        Sent {
            title: Text::text("x"),
            progress,
            style: Style::default(),
            effects: Effects::default(),
        }
    }

    /// An id is a pure function of the bar and nothing else, so there is no
    /// table to keep in step and two bars can never collide.
    #[test]
    fn a_bars_id_is_its_entity_under_one_namespace() {
        assert_eq!(bar_uuid(Entity(7)), bar_uuid(Entity(7)));
        assert_ne!(bar_uuid(Entity(7)), bar_uuid(Entity(8)));
        // The low half is the entity and the high half is not, which is what
        // keeps these away from ids a game mints for its own bars.
        assert_eq!(bar_uuid(Entity(7)).0 & u128::from(u64::MAX), 7);
        assert_eq!(bar_uuid(Entity(7)).0 >> 64, u128::from(NAMESPACE));
    }

    /// A fill nobody can draw is drawn as empty rather than sent as it is.
    ///
    /// `f32::clamp` propagates `NaN`, so the clamp alone is not enough: a
    /// `NaN` on the wire is a bar the client draws at whatever width the
    /// comparison happens to give, forever.
    #[test]
    fn a_fill_that_is_not_a_fraction_is_empty() {
        assert!((fill(0.25) - 0.25).abs() < 1e-9);
        assert!((fill(4.0) - 1.0).abs() < 1e-9);
        assert!((fill(-1.0)).abs() < 1e-9);
        assert!((fill(f32::NAN)).abs() < 1e-9);
        assert!((fill(f32::INFINITY)).abs() < 1e-9);
    }

    /// Two clients on one bar are two rows of the drive query.
    ///
    /// The audience loop rests entirely on a `(Sent, *)` term producing one
    /// result per target rather than one per entity, and if it produced one
    /// per entity the second viewer's bar would silently never update. Nothing
    /// else in this repo iterates a wildcard pair for its data, so the
    /// assumption is pinned here rather than discovered in a match.
    #[test]
    fn a_wildcard_pair_term_visits_every_target_not_just_the_first() {
        let world = World::new();
        world.component::<Sent>();
        world.component::<BossBar>();

        let alice = world.entity();
        let bob = world.entity();
        let bar = world
            .entity()
            .add(id::<BossBar>())
            .set_first(state(0.25), alice)
            .set_first(state(0.75), bob);

        let query = world
            .query::<&mut (Sent, flecs::Wildcard)>()
            .with(id::<BossBar>())
            .build();

        let mut seen = Vec::new();
        query.each_iter(|it, row, sent| {
            seen.push((
                it.entity(row).id(),
                it.pair(0).second_id().id(),
                sent.progress,
            ));
        });
        seen.sort_by_key(|(_, viewer, _)| *viewer);

        assert_eq!(seen, vec![
            (bar.id(), alice.id(), 0.25),
            (bar.id(), bob.id(), 0.75)
        ]);
    }

    /// Deleting a viewer takes away what that viewer was told and leaves
    /// everybody else's bar alone.
    ///
    /// This is flecs's default cleanup and the whole teardown path is built on
    /// it, so it is written down as a claim rather than assumed: the other
    /// default, `Delete`, would mean one player quitting removed the bar from
    /// every screen.
    #[test]
    fn a_viewer_leaving_removes_only_their_own_record() {
        let world = World::new();
        world
            .component::<Sent>()
            .add_trait::<(flecs::OnDeleteTarget, flecs::Remove)>();

        let alice = world.entity();
        let bob = world.entity();
        let bar = world
            .entity()
            .set_first(state(0.25), alice)
            .set_first(state(0.75), bob);

        alice.destruct();

        assert!(bar.is_alive(), "one viewer leaving destroyed the bar");
        assert!(bar.has((id::<Sent>(), bob.id())));
        let mut targets = 0;
        bar.each_target(id::<Sent>(), |_| targets += 1);
        assert_eq!(targets, 1, "the departed viewer's record outlived them");
    }
}

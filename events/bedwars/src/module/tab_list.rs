//! The tab list header, and the same numbers on the console.
//!
//! The packet itself is not sent from here. `hyperion::egress::tab_list` owns
//! the `TabList` singleton and broadcasts it when it changes; this module
//! writes the header half and lets that happen. Before that split existed both
//! halves were built and broadcast here, unconditionally, to every player,
//! twenty times a second.
//!
//! The tick-time averages are still worth showing next to the tick *rate* the
//! footer carries: a rate says whether the server kept up, and the three
//! averages say how much headroom it had while doing it.

use flecs_ecs::{
    core::{SystemAPI, World, WorldGet},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    egress::tab_list::{TabList, TabListComponentsModule, Text},
    hyperion_minecraft_proto::text::NamedColor,
    net::Compose,
};
use tracing::{info, info_span};

/// Named for the crate it belongs to, not for what it does, and that is
/// load bearing: flecs names a module entity after the **last segment** of
/// its type path and nothing else, so this and
/// `hyperion::egress::tab_list::TabListModule` were one name in one flat
/// namespace. A dev build aborts on the collision --
/// `entity symbol inconsistent: bedwars::module::tab_list::TabListModule
/// (provided) vs. hyperion::egress::tab_list::TabListModule (existing)` --
/// and a release build, where that assert is compiled out, quietly treats
/// the import as already done and installs none of this.
#[derive(Component)]
pub struct BedwarsTabListModule;

/// One console line, and one header rebuild, per second at the 20 Hz tick
/// rate.
///
/// The header is rate limited rather than written every tick because a
/// millisecond average to two decimals moves on every single one, and
/// `tab_list_sync` sends whenever the text changes: writing it at tick rate
/// would put the per-tick broadcast straight back. Nobody reads a number that
/// changes twenty times a second anyway.
const TICKS_PER_LOG: u32 = 20;

/// How many samples the longest window holds.
const WINDOW_TICKS: usize = 20 * 60;

impl Module for BedwarsTabListModule {
    fn module(world: &World) {
        // The header is written into a component this module does not own, so
        // the module that registers it is imported rather than assumed.
        world.import::<TabListComponentsModule>();

        let mode = env!("RUN_MODE");

        let mut tick_times = Vec::with_capacity(WINDOW_TICKS);
        let mut last_frame_time_total = 0.0;
        let mut ticks_since_log = 0u32;

        system!("stats", world, &Compose).each_iter(move |it, _, compose| {
            let span = info_span!("stats");
            let _enter = span.enter();
            let world = it.world();
            let player_count = compose
                .global()
                .player_count
                .load(std::sync::atomic::Ordering::Relaxed);

            let info = world.info();
            let current_frame_time_total = info.frame_time_total;

            let ms_per_tick = (current_frame_time_total - last_frame_time_total) * 1000.0;
            last_frame_time_total = current_frame_time_total;

            tick_times.push(ms_per_tick);
            if tick_times.len() > WINDOW_TICKS {
                tick_times.remove(0);
            }

            let avg_s05 = tick_times.iter().rev().take(20 * 5).sum::<f32>() / (20.0 * 5.0);
            let avg_s15 = tick_times.iter().rev().take(20 * 15).sum::<f32>() / (20.0 * 15.0);
            let avg_s60 = tick_times.iter().sum::<f32>() / tick_times.len() as f32;

            ticks_since_log += 1;
            if ticks_since_log < TICKS_PER_LOG {
                return;
            }
            ticks_since_log = 0;

            // Components and not `§` markup, which is the rule
            // `hyperion::egress::boss_bar` states and the same `Text` type
            // carries here: a colour a caller can smuggle in as text is a
            // colour nothing can check.
            let header = Text::text("").extend([
                Text::text(mode).color(NamedColor::Aqua),
                Text::text("\n"),
                Text::text(format!("µ/5s: {avg_s05:.2} ms")).color(NamedColor::Green),
                Text::text(" | "),
                Text::text(format!("µ/15s: {avg_s15:.2} ms")).color(NamedColor::Yellow),
                Text::text(" | "),
                Text::text(format!("µ/1m: {avg_s60:.2} ms")).color(NamedColor::Red),
            ]);
            world.get::<&mut TabList>(|list| {
                if list.header != header {
                    list.header = header;
                }
            });

            // The tab list carries this already, but an operator watching the
            // console has no other way to see whether anyone is connected or how
            // the tick budget is holding up.
            info!(
                "{player_count} players online | tick µ/5s {avg_s05:.2} ms, µ/15s {avg_s15:.2} \
                 ms, µ/1m {avg_s60:.2} ms"
            );
        });
    }
}

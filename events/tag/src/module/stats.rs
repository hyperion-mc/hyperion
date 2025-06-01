use std::time::Instant;

use bevy::prelude::*;
use hyperion::{
    net::Compose,
    valence_protocol::{packets::play, text::IntoText},
};
use tracing::info_span;

#[derive(Resource)]
struct TickStartTime(Instant);

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    #[allow(clippy::excessive_nesting)]
    fn build(&self, app: &mut App) {
        let mode = env!("RUN_MODE");

        let mut tick_times = Vec::with_capacity(20 * 60); // 20 ticks per second, 60 seconds

        app.insert_resource(TickStartTime(Instant::now()));

        // TODO: There may be some other systems outside of FixedUpdate which should be included in
        // the total tick time
        app.add_systems(FixedPreUpdate, |mut start: ResMut<'_, TickStartTime>| {
            start.0 = Instant::now();
        });

        app.add_systems(
            FixedPostUpdate,
            move |compose: Res<'_, Compose>, start: Res<'_, TickStartTime>| {
                let span = info_span!("stats");
                let _enter = span.enter();
                let player_count = compose
                    .global()
                    .player_count
                    .load(std::sync::atomic::Ordering::Relaxed);

                let ms_per_tick = start.0.elapsed().as_secs_f32() * 1000.0;

                tick_times.push(ms_per_tick);
                if tick_times.len() > 20 * 60 {
                    tick_times.remove(0);
                }

                let avg_s05 = tick_times.iter().rev().take(20 * 5).sum::<f32>() / (20.0 * 5.0);
                let avg_s15 = tick_times.iter().rev().take(20 * 15).sum::<f32>() / (20.0 * 15.0);
                let avg_s60 = tick_times.iter().sum::<f32>() / tick_times.len() as f32;

                let title = format!(
                    "§b{mode}§r\n§aµ/5s: {avg_s05:.2} ms §r| §eµ/15s: {avg_s15:.2} ms §r| §cµ/1m: \
                     {avg_s60:.2} ms"
                );

                let footer = format!("§d§l{player_count} players online");

                let pkt = play::PlayerListHeaderS2c {
                    header: title.into_cow_text(),
                    footer: footer.into_cow_text(),
                };

                compose.broadcast(&pkt).send().unwrap();
            },
        );
    }
}

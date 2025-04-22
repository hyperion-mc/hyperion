use flecs_ecs::{
    core::{QueryBuilderImpl, SystemAPI, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    net::Compose,
    valence_protocol::{packets::play, text::IntoText},
};
use tracing::info_span;

#[derive(Component)]
pub struct StatsModule;

impl Module for StatsModule {
    #[allow(clippy::excessive_nesting)]
    fn module(world: &World) {
        let mode = env!("RUN_MODE");

        let mut tick_times = Vec::with_capacity(20 * 60); // 20 ticks per second, 60 seconds
        let mut last_frame_time_total = 0.0;

        system!("stats", world, &Compose($)).each_iter(move |it, _, compose| {
            let span = info_span!("stats");
            let _enter = span.enter();
            let system = it.system();
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

            compose.broadcast(&pkt, system).send().unwrap();
        });
    }
}

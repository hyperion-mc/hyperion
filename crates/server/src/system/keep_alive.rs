use std::time::Instant;

use evenio::prelude::*;
use tracing::{instrument, trace};

use crate::{
    components::KeepAlive,
    events::{Gametick, KickPlayer},
    global::Global,
    net,
    net::{IoBuf, IoBufs, Packets},
    system::player_join_world::send_keep_alive,
};

#[instrument(skip_all, level = "trace")]
pub fn keep_alive(
    gametick: ReceiverMut<Gametick>,
    global: Single<&Global>,
    mut io: Single<&mut IoBufs>,
    mut fetcher: Fetcher<(EntityId, &mut KeepAlive, &mut Packets)>,
    mut s: Sender<KickPlayer>,
    mut compressor: Single<&mut net::Compressor>,
) {
    let mut gametick = gametick.event;
    let scratch = &mut gametick.scratch;
    let compressor = compressor.one();

    fetcher.iter_mut().for_each(|(id, keep_alive, packets)| {
        let Some(sent) = &mut keep_alive.last_sent else {
            keep_alive.last_sent = Some(Instant::now());
            return;
        };

        // if we haven't sent a keep alive packet in 5 seconds, and a keep alive hasn't already
        // been sent and hasn't been responded to, send one
        let elapsed = sent.elapsed();

        if elapsed > global.keep_alive_timeout {
            s.send(KickPlayer {
                target: id,
                reason: "keep alive timeout".into(),
            });
            return;
        }

        if !keep_alive.unresponded && elapsed.as_secs() >= 5 {
            *sent = Instant::now();

            // todo: handle and disconnect
            let mut scratch = scratch.one();
            send_keep_alive(packets, io.one(), scratch, compressor).unwrap();

            trace!("keep alive");
        }
    });
}

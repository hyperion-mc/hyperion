use std::sync::{Arc, atomic, atomic::AtomicBool};

use anyhow::bail;
use bytes::Bytes;
use slotmap::{KeyData, new_key_type};

use crate::cache::ExclusionsManager;

new_key_type! {
    pub struct PlayerId;
}

impl From<u64> for PlayerId {
    fn from(id: u64) -> Self {
        let raw = KeyData::from_ffi(id);
        Self::from(raw)
    }
}

pub struct OrderedBytes {
    /// The order number for this packet. Packets can be received in any order,
    /// but will be reordered before being written to ensure monotonically increasing order.
    /// Each packet is assigned a sequence number that determines its final ordering.
    pub order: u32,
    pub offset: u32,
    pub data: Bytes,
    pub exclusions: Option<Arc<ExclusionsManager>>,
}

impl OrderedBytes {
    pub const DEFAULT: Self = Self {
        order: 0,
        offset: 0,
        data: Bytes::from_static(b""),
        exclusions: None,
    };
    pub const FLUSH: Self = Self {
        order: u32::MAX,
        offset: 0,
        data: Bytes::from_static(b""),
        exclusions: None,
    };

    pub const fn is_flush(&self) -> bool {
        self.order == u32::MAX
    }

    pub const fn no_order(data: Bytes) -> Self {
        Self {
            order: 0,
            offset: 0,
            data,
            exclusions: None,
        }
    }

    pub const fn with_exclusions(
        order: u32,
        data: Bytes,
        exclusions: Arc<ExclusionsManager>,
    ) -> Self {
        Self {
            order,
            offset: 0,
            data,
            exclusions: Some(exclusions),
        }
    }
}

impl PartialEq<Self> for OrderedBytes {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order
    }
}

impl Eq for OrderedBytes {}

impl PartialOrd<Self> for OrderedBytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedBytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order.cmp(&other.order)
    }
}

#[derive(Debug)]
pub struct PlayerHandle {
    writer: kanal::AsyncSender<OrderedBytes>,

    /// Whether the player is allowed to send broadcasts.
    ///
    /// This exists because the player is not automatically in the play state,
    /// and if they are not in the play state and they receive broadcasts,
    /// they will get packets that it deems are invalid because the broadcasts are using the play
    /// state and play IDs.
    can_receive_broadcasts: AtomicBool,
}

impl PlayerHandle {
    #[must_use]
    pub const fn new(writer: kanal::AsyncSender<OrderedBytes>) -> Self {
        Self {
            writer,
            can_receive_broadcasts: AtomicBool::new(false),
        }
    }

    pub fn shutdown(&self) {
        // Ignore error for if the channel is already closed
        let _ = self.writer.close();
    }

    pub fn enable_receive_broadcasts(&self) {
        self.can_receive_broadcasts
            .store(true, atomic::Ordering::Relaxed);
    }

    pub fn can_receive_broadcasts(&self) -> bool {
        self.can_receive_broadcasts.load(atomic::Ordering::Relaxed)
    }

    pub fn send(&self, ordered_bytes: OrderedBytes) -> anyhow::Result<()> {
        match self.writer.try_send(ordered_bytes) {
            Ok(true) => Ok(()),

            Ok(false) => {
                let is_full = self.writer.is_full();
                self.shutdown();
                bail!("failed to send packet to player, channel is full: {is_full}");
            }
            Err(e) => {
                self.shutdown();
                bail!("failed to send packet to player: {e}");
            }
        }
    }
}

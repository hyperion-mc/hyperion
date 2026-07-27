//! Broadcasting the blocks that changed inside a chunk already sent.
//!
//! A column is encoded once and cached, so a block someone breaks cannot be
//! folded back into it. Instead each section keeps the indices it has changed
//! and those go out as `section_blocks_update`, which is one packet per
//! section however many blocks moved.
//!
//! Two variants, because the two questions are different. A player who has
//! just walked into range needs every change since the world loaded
//! ([`DeltaPacket`]); a player already watching needs only this tick's
//! ([`DeltaDrainPacket`], which clears the tick set as it goes).

use std::io::Write;

use glam::IVec2;
use hyperion_minecraft_proto::{
    Encode, Writer,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::chunk::{BlockChange, SectionBlocksUpdate, SectionPos},
};
use valence_generated::block::BlockState;

use crate::{
    PacketBundle,
    simulation::blocks::{
        chunk::{Column, START_Y},
        loader::parse::section::Section,
        translate,
    },
};

/// Turn the changed indices of one section into the packet body.
///
/// A section index is `y << 8 | z << 4 | x`, the same packing
/// `SectionPos.sectionRelative` reads back out, so the three coordinates come
/// straight out of it.
fn changes(section: &Section, indices: impl Iterator<Item = u32>) -> Vec<BlockChange> {
    indices
        .map(|index| {
            let raw = section.block_states.get(index as usize);
            let state = BlockState::from_raw(raw).expect("stored state is a 1.20.1 state");
            // Each coordinate is masked to its four bits before the cast.
            BlockChange {
                x: (index & 0xF) as u8,
                y: ((index >> 8) & 0xF) as u8,
                z: ((index >> 4) & 0xF) as u8,
                state: translate::block_state(state),
            }
        })
        .collect()
}

fn write_update(mut write: impl Write, update: &SectionBlocksUpdate) -> anyhow::Result<()> {
    let mut writer = Writer::new();
    writer.var_int(PacketId::SectionBlocksUpdate.to_raw());
    update.encode(&mut writer)?;
    write.write_all(writer.as_slice())?;
    Ok(())
}

/// This tick's changes in one section, cleared as they are written.
#[derive(derive_more::Debug)]
pub struct DeltaDrainPacket<'a> {
    position: SectionPos,
    #[debug(skip)]
    section: &'a mut Section,
}

impl PacketBundle for DeltaDrainPacket<'_> {
    fn encode_including_ids(self, write: impl Write) -> anyhow::Result<()> {
        let update = SectionBlocksUpdate {
            section: self.position,
            changes: changes(self.section, self.section.changed_since_last_tick.iter()),
        };
        write_update(write, &update)?;
        self.section.reset_tick_deltas();
        Ok(())
    }
}

/// Every change this section has seen since the column was loaded.
#[derive(derive_more::Debug)]
pub struct DeltaPacket<'a> {
    position: SectionPos,
    #[debug(skip)]
    section: &'a Section,
}

impl PacketBundle for DeltaPacket<'_> {
    fn encode_including_ids(self, write: impl Write) -> anyhow::Result<()> {
        let update = SectionBlocksUpdate {
            section: self.position,
            changes: changes(self.section, self.section.changed.iter()),
        };
        write_update(write, &update)
    }
}

impl Column {
    pub fn delta_drain_packets(&mut self) -> impl Iterator<Item = DeltaDrainPacket<'_>> + '_ {
        let IVec2 { x, y: z } = self.position;

        self.data
            .sections
            .iter_mut()
            .enumerate()
            .filter(|(_, section)| !section.changed_since_last_tick.is_empty())
            .map(move |(i, section)| {
                let y = i32::try_from(i).unwrap();
                let y = y + i32::from(START_Y >> 4);

                DeltaDrainPacket {
                    position: SectionPos::new(x, y, z),
                    section,
                }
            })
    }

    pub fn original_delta_packets(&self) -> impl Iterator<Item = DeltaPacket<'_>> + '_ {
        let IVec2 { x, y: z } = self.position;

        self.data
            .sections
            .iter()
            .enumerate()
            .filter(|(_, section)| !section.changed.is_empty())
            .map(move |(i, section)| {
                let y = i32::try_from(i).unwrap();
                let y = y + i32::from(START_Y >> 4);

                DeltaPacket {
                    position: SectionPos::new(x, y, z),
                    section,
                }
            })
    }
}

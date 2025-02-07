use std::fmt::Debug;

use bytes::Bytes;
use glam::{IVec2, IVec3};
use valence_generated::block::BlockState;
use valence_server::layer::chunk::Chunk;

use super::loader::parse::ColumnData;
use crate::simulation::blocks::loader::parse::section::Section;

pub const START_Y: i16 = -64;

#[repr(Rust, packed)]
#[derive(Copy, Clone)]
pub struct OnChange {
    xz: u8, // 1
    y: u16, // 2
}

impl Debug for OnChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnChange")
            .field("x", &self.x())
            .field("z", &self.z())
            .field("y", &self.y())
            .finish()
    }
}

impl OnChange {
    #[must_use]
    pub const fn new(x: u8, y: u16, z: u8) -> Self {
        Self {
            xz: (x << 4) | (z & 0b1111),
            y,
        }
    }

    #[must_use]
    pub const fn x(&self) -> u8 {
        self.xz >> 4
    }

    #[must_use]
    pub const fn z(&self) -> u8 {
        self.xz & 0b1111
    }

    #[must_use]
    pub const fn y(&self) -> u16 {
        self.y
    }
}

const _: () = assert!(size_of::<OnChange>() == 3);

mod packet;

/// A chunk which has been loaded into memory.
#[derive(Debug)]
pub struct Column {
    /// The raw (usually compressed) bytes of the chunk that are sent to the client via the Minecraft protocol.
    pub base_packet_bytes: Bytes,

    /// The actual chunk data that is "uncompressed". It uses a palette to store the actual data. This is usually used
    /// for obtaining the actual data from the chunk such as getting the block state of a block at a given position.
    pub data: ColumnData,

    pub position: IVec2,
}

fn y_index(y: i16) -> u16 {
    u16::try_from(y - START_Y).unwrap()
}

impl Column {
    pub const fn new(base_packet_bytes: Bytes, data: ColumnData, position: IVec2) -> Self {
        Self {
            base_packet_bytes,
            data,
            position,
        }
    }

    pub fn sections(&self) -> impl Iterator<Item = (IVec3, &Section)> + '_ {
        let column_start_position = IVec3::new(
            self.position.x << 4,
            i32::from(START_Y),
            self.position.y << 4,
        );

        self.data
            .sections
            .iter()
            .enumerate()
            .map(move |(section_idx, section)| {
                let section_y = i32::try_from(section_idx).unwrap();
                let section_y = section_y * 16;
                let section_start = column_start_position + IVec3::new(0, section_y, 0);

                (section_start, section)
            })
    }

    pub fn blocks_in_range(
        &self,
        min_y: i16,
        max_y: i16,
    ) -> impl Iterator<Item = (IVec3, BlockState)> + '_ {
        let min_y_idx = y_index(min_y);
        let max_y_idx = y_index(max_y);
        let min_section = (min_y_idx >> 4) as usize;
        let max_section = (max_y_idx >> 4) as usize;

        (min_section..=max_section)
            .flat_map(move |section_idx| {
                let section = &self.data.sections[section_idx];
                section.blocks_states().map(move |(pos, state)| {
                    let section_y = u16::try_from(section_idx).unwrap();

                    debug_assert!(pos.y < 16, "pos.y is {}", pos.y);

                    let y = (section_y * 16) + pos.y;
                    (
                        IVec3::new(
                            i32::from(pos.x) + self.position.x * 16,
                            i32::from(y) + i32::from(START_Y),
                            i32::from(pos.z) + self.position.y * 16,
                        ),
                        state,
                    )
                })
            })
            .filter(move |(pos, _)| {
                let y = pos.y;
                y >= i32::from(min_y) && y <= i32::from(max_y)
            })
    }

    pub fn bytes(&self) -> Bytes {
        self.base_packet_bytes.clone()
    }

    #[expect(unused, reason = "might be useful in the future")]
    fn set_block_internal(&mut self, x: u8, y: u16, z: u8, state: BlockState) {
        self.data
            .set_block(u32::from(x), u32::from(y), u32::from(z), state);
    }

    fn get_block_internal(&self, x: u8, y: u16, z: u8) -> u16 {
        self.data
            .block_state(u32::from(x), u32::from(y), u32::from(z))
            .to_raw()
    }

    #[expect(unused, reason = "might be useful in the future")]
    fn get_block(&self, x: u8, y: u16, z: u8) -> BlockState {
        BlockState::from_raw(self.get_block_internal(x, y, z)).unwrap()
    }
}

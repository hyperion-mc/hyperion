#![allow(
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::cast_possible_wrap,
    reason = "this is code from valence; we plan to improve it in the future"
)]

use std::io::Write;

use valence_protocol::{Encode, VarInt};

use crate::{Data, LEN, PalettedContainer};

const fn bit_width(n: usize) -> usize {
    n.bit_width() as usize
}

const fn compact_u64s_len(vals_count: usize, bits_per_val: usize) -> usize {
    let vals_per_u64 = 64 / bits_per_val;
    vals_count.div_ceil(vals_per_u64)
}

fn encode_compact_u64s(
    mut w: impl Write,
    mut vals: impl Iterator<Item = u64>,
    bits_per_val: usize,
) -> anyhow::Result<()> {
    debug_assert!(bits_per_val <= 64);

    let vals_per_u64 = 64 / bits_per_val;

    loop {
        let mut n = 0;
        for i in 0..vals_per_u64 {
            match vals.next() {
                Some(val) => {
                    debug_assert!(val < 2_u128.pow(bits_per_val as _) as _);
                    n |= val << (i * bits_per_val);
                }
                None if i > 0 => return n.encode(&mut w),
                None => return Ok(()),
            }
        }
        n.encode(&mut w)?;
    }
}

impl PalettedContainer {
    pub fn encode_mc_format<W, F>(
        &self,
        mut writer: W,
        mut to_bits: F,
        min_indirect_bits: usize,
        max_indirect_bits: usize,
        direct_bits: usize,
    ) -> anyhow::Result<()>
    where
        W: Write,
        F: FnMut(Data) -> u64,
    {
        debug_assert!(min_indirect_bits <= 4);
        debug_assert!(min_indirect_bits <= max_indirect_bits);
        debug_assert!(max_indirect_bits <= 64);
        debug_assert!(direct_bits <= 64);

        match self {
            Self::Single(val) => {
                // Bits per entry
                0_u8.encode(&mut writer)?;
                // Palette
                VarInt(to_bits(*val) as i32).encode(&mut writer)?;
                // Number of longs
                VarInt(0).encode(writer)?;
            }
            Self::Indirect(ind) => {
                let bits_per_entry = min_indirect_bits.max(bit_width(ind.palette_len as usize - 1));
                // Encode as direct if necessary.
                if bits_per_entry > max_indirect_bits {
                    // Bits per entry
                    (direct_bits as u8).encode(&mut writer)?;
                    // Number of longs in data array.
                    VarInt(compact_u64s_len(LEN, direct_bits) as _).encode(&mut writer)?;
                    // Data array
                    encode_compact_u64s(
                        writer,
                        (0..LEN).map(|i| to_bits(unsafe { ind.get_unchecked(i) })),
                        direct_bits,
                    )?;
                } else {
                    // Bits per entry
                    (bits_per_entry as u8).encode(&mut writer)?;
                    // Palette len
                    VarInt(ind.palette.len() as i32).encode(&mut writer)?;
                    // Palette
                    for val in &ind.palette {
                        VarInt(to_bits(*val) as i32).encode(&mut writer)?;
                    }
                    // Number of longs in data array.
                    VarInt(compact_u64s_len(LEN, bits_per_entry) as _).encode(&mut writer)?;
                    // Data array
                    encode_compact_u64s(
                        writer,
                        ind.indices()
                            // .flat_map(|byte| [byte & 0b1111, byte >> 4])
                            .map(u64::from)
                            .take(LEN),
                        bits_per_entry,
                    )?;
                }
            }
            Self::Direct(dir) => {
                // Bits per entry
                (direct_bits as u8).encode(&mut writer)?;
                // Number of longs in data array.
                VarInt(compact_u64s_len(LEN, direct_bits) as _).encode(&mut writer)?;
                // Data array
                encode_compact_u64s(writer, dir.iter().copied().map(&mut to_bits), direct_bits)?;
            }
        }
        Ok(())
    }
}

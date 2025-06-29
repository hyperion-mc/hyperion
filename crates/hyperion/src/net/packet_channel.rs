use std::{
    cell::SyncUnsafeCell,
    mem::{MaybeUninit, size_of},
    num::NonZeroU32,
    ops::{Deref, Range},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use arc_swap::ArcSwapOption;
use bevy::prelude::*;
use more_asserts::debug_assert_le;
use valence_protocol::MAX_PACKET_SIZE;

/// Reference counted fragment. Fragments are a fixed-size block of data which may contain one or
/// more packets.
struct Fragment {
    /// Points to the next fragment, if available. Once this is set to `Some`, this fragment's `read_cursor` will never be
    /// updated and `next` will never be modified.
    next: ArcSwapOption<Fragment>,

    /// Bytes `0..read_cursor` in the `data` field have been initialized, can be read, will never
    /// be modified again, and contain entire packets.
    read_cursor: AtomicUsize,

    /// Fragment id. The first fragment will have an id of 0, and the next fragment will have an id
    /// that is one greater than the previous fragment's id.
    id: usize,

    // TODO: Consider using unsized types to avoid this extra Box allocation
    /// Stores packets in the following format:
    /// - a `u32` for the packet size encoded as native-endian bytes
    /// - packet bytes
    ///
    /// No padding is present. An individual packet will always be stored in exactly one fragment; a packet
    /// will never be stored across multiple [`Fragment`]s.
    data: Box<[SyncUnsafeCell<MaybeUninit<u8>>]>,
}

impl Fragment {
    fn new(size: usize, id: usize) -> Self {
        let data: Box<[MaybeUninit<u8>]> = Box::new_uninit_slice(size);
        // SAFETY: MaybeUninit<u8> and SyncUnsafeCell<MaybeUninit<u8>> have the same layout
        let data: Box<[SyncUnsafeCell<MaybeUninit<u8>>]> =
            unsafe { Box::from_raw(Box::into_raw(data) as *mut _) };
        Self {
            next: ArcSwapOption::from(None),
            read_cursor: AtomicUsize::new(0),
            id,
            data,
        }
    }

    /// # Safety
    /// `range` must be within `0..read_cursor`
    unsafe fn read_unchecked(&self, range: Range<usize>) -> &[u8] {
        #[cfg(debug_assertions)]
        if !range.is_empty() {
            let read_cursor = self.read_cursor.load(Ordering::Relaxed);
            debug_assert_le!(
                range.end,
                read_cursor,
                "attempted to read bytes {range:?} which is outside of readable fragment range \
                 (0..{read_cursor})"
            );
        }

        // SAFETY: Caller ensures that this should be in bounds
        let data: &[SyncUnsafeCell<MaybeUninit<u8>>] = unsafe { self.data.get_unchecked(range) };

        // SAFETY: SyncUnsafeCell<MaybeUninit<u8>> has the same layout as u8. The caller ensures
        // that these bytes are ready for reading, meaning that they have been properly
        // initialized. Once fragment bytes are initialized, they are never written to again, so
        // creating a shared reference to them is okay.
        let data: &[u8] = unsafe { std::slice::from_raw_parts(data.as_ptr().cast(), data.len()) };

        data
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SendState {
    ReadLen { current_len: u32, bit_offset: u32 },
    ReadData { remaining: NonZeroU32 },
    Closed,
}

pub struct Sender {
    current: Arc<Fragment>,
    write_cursor: usize,
    default_fragment_size: usize,
    send_state: SendState,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SendError {
    ZeroLengthPacket,
    TooLargePacket,
    AlreadyClosed,
}

impl Sender {
    /// Sends a packet.
    ///
    /// If a send error occurs, the reader will be able to read all packets that were before the
    /// malfromed packet.
    pub fn send(&mut self, mut data: &[u8]) -> Result<(), SendError> {
        if data.is_empty() {
            return if self.send_state == SendState::Closed {
                Err(SendError::AlreadyClosed)
            } else {
                Ok(())
            };
        }

        let mut last_full_packet_cursor = None;
        let mut result = Ok(());
        while let Some(first_byte) = data.first() {
            match self.send_state {
                SendState::ReadLen {
                    mut current_len,
                    bit_offset,
                } => {
                    // TODO: Optimize this when the entire len is available in data
                    const DATA_MASK: u8 = 0b0111_1111;
                    const CONTINUE_BIT: u8 = !DATA_MASK;

                    current_len |= u32::from(first_byte & DATA_MASK) << bit_offset;

                    if current_len >= MAX_PACKET_SIZE as u32 {
                        self.send_state = SendState::Closed;
                        result = Err(SendError::TooLargePacket);
                        break;
                    }

                    if first_byte & CONTINUE_BIT == 0 {
                        // Stop reading size
                        let Some(remaining) = NonZeroU32::new(current_len) else {
                            self.send_state = SendState::Closed;
                            result = Err(SendError::ZeroLengthPacket);
                            break;
                        };

                        self.send_state = SendState::ReadData { remaining };

                        // Determine if a new fragment needs to be allocated
                        let total_len = size_of::<u32>() + current_len as usize;
                        if self.current.data.len() - self.write_cursor < total_len {
                            // Allocate a new fragment
                            // SAFETY: Sender is not closed
                            unsafe {
                                self.new_fragment(std::cmp::max(
                                    total_len,
                                    self.default_fragment_size,
                                ));
                            }

                            // The read cursor in the previous fragment will have already been
                            // updated.
                            last_full_packet_cursor = None;
                        }

                        // Write the size to the current fragment
                        // SAFETY: The code above ensures that the current fragment has enough
                        // space to store the len
                        unsafe {
                            self.write(&current_len.to_ne_bytes());
                        }
                    } else {
                        // Continue bit is set
                        self.send_state = SendState::ReadLen {
                            current_len,
                            bit_offset: bit_offset + 7,
                        };
                    }
                    data = &data[1..];
                }
                SendState::ReadData { remaining } => {
                    // `len` describes the number of bytes that should be copied from `data` to
                    // the current packet.
                    // Doing a saturating cast of `data.len()` from `usize` to `u32` is okay
                    // because the packet size must be less than `u32`, so if `data.len() > u32::MAX`,
                    // it is not possible for the bytes after the `u32::MAX` index to be in the current packet.
                    // `len` is guaranteed to be nonzero because `remaining` is a `NonZeroU32`,
                    // and `data.len()` must be nonzero because `data.first()` succeeded.
                    let len = std::cmp::min(
                        remaining.get(),
                        u32::try_from(data.len()).unwrap_or(u32::MAX),
                    );

                    let len_usize = len as usize;

                    let bytes = &data[..len_usize];

                    // SAFETY: When the length of the packet was initially read, the code ensured
                    // that the fragment has enough space to store the packet length and the packet
                    // data and allocated a new fragment is needed. In addition, `bytes` is
                    // guaranteed to not be empty because `len` is nonzero.
                    unsafe {
                        self.write(bytes);
                    }

                    if let Some(new_remaining) = NonZeroU32::new(remaining.get() - len) {
                        // The packet is still incomplete and needs more bytes from future send
                        // calls.
                        self.send_state = SendState::ReadData {
                            remaining: new_remaining,
                        };
                        break;
                    }

                    // The full packet has been read. The next iteration of this loop will
                    // begin sending the next packet.
                    last_full_packet_cursor = Some(self.write_cursor);
                    data = &data[len_usize..];
                    self.send_state = SendState::ReadLen {
                        current_len: 0,
                        bit_offset: 0,
                    };
                }
                SendState::Closed => {
                    result = Err(SendError::AlreadyClosed);
                    break;
                }
            }
        }

        if let Some(last_full_packet_cursor) = last_full_packet_cursor {
            // SAFETY: The last full packet cursor points to data that has already been initialized
            // with complete packets
            unsafe {
                self.update_read_cursor(last_full_packet_cursor);
            }
        }

        result
    }

    /// Writes bytes to the current fragment
    ///
    /// # Safety
    /// `source` must not be empty.
    /// There must be enough space in the current fragment to write `source`.
    /// This sender must not be closed.
    unsafe fn write(&mut self, source: &[u8]) {
        debug_assert_ne!(
            self.send_state,
            SendState::Closed,
            "cannot write to a closed sender"
        );

        if source.is_empty() {
            // SAFETY: Caller must ensure that source is not empty. This requirement is useful to
            // allow the compiler to optimize out a check. If source was allowed to be empty,
            // it could point to one past the end of an allocation, which does not point to a
            // valid object and it would not be allowed to memcpy that source to the fragment even
            // with a size of zero.
            // Without this non-empty requirement, the compiler would introduce an extra check to
            // see if it is empty.
            unsafe { std::hint::unreachable_unchecked() };
        }

        // SAFETY: Caller must ensure there is enough space in the current fragment to write the
        // data.
        let dest: &[SyncUnsafeCell<MaybeUninit<u8>>] = unsafe {
            self.current
                .data
                .get_unchecked(self.write_cursor..(self.write_cursor + source.len()))
        };

        // Copying the bytes one at a time is required because using SyncUnsafeCell::{raw,}get is the
        // only legal way to get the underlying *mut MaybeUninit<u8>. However, the compiler seems
        // to be able to successfully optimize this into a memcpy in release mode.
        for (dest_byte, source_byte) in dest.iter().zip(source.iter()) {
            // SAFETY: The reader cannot be reading these bytes because the read cursor points to
            // an index before these bytes.
            unsafe { (*dest_byte.get()).write(*source_byte) };
        }

        self.write_cursor += source.len();
    }

    /// Allocates a new fragment with a given size. The current fragment is then set to this new
    /// fragment.
    ///
    /// # Safety
    /// The sender must not be closed.
    unsafe fn new_fragment(&mut self, size: usize) {
        debug_assert_ne!(
            self.send_state,
            SendState::Closed,
            "cannot allocate a new fragment from a closed sender"
        );

        // Update the read cursor of the current fragment
        // SAFETY: Setting the read cursor to the write cursor is allowed
        unsafe {
            self.update_read_cursor(self.write_cursor);
        }

        self.write_cursor = 0;

        // TODO: There should be a channel of freed fragments so that this can reuse allocations
        // Allocate a new fragment
        let next_fragment = Arc::new(Fragment::new(size, self.current.id + 1));

        self.current.next.store(Some(next_fragment.clone()));
        self.current = next_fragment;
    }

    /// # Safety
    /// The `read_cursor` parameter must be less than or equal to `self.write_cursor`.
    /// The bytes before the `read_cursor` must be complete packets.
    /// The sender must not be closed.
    unsafe fn update_read_cursor(&mut self, read_cursor: usize) {
        debug_assert_ne!(
            self.send_state,
            SendState::Closed,
            "cannot update read cursor from a closed sender"
        );
        debug_assert_le!(
            read_cursor,
            self.write_cursor,
            "read_cursor must be less than or equal to write_cursor"
        );
        self.current
            .read_cursor
            .store(read_cursor, Ordering::Release);
    }
}

#[derive(Component)]
pub struct Receiver {
    current: Arc<Fragment>,
    read_cursor: usize,
}

impl Receiver {
    /// Receives next availale packet
    pub fn try_recv(&mut self) -> Option<RawPacket> {
        loop {
            let fragment_read_cursor = self.current.read_cursor.load(Ordering::Acquire);

            if self.read_cursor == fragment_read_cursor {
                if let Some(next_fragment) = self.current.next.load_full() {
                    // The next fragment is available. This needs to check if the read cursor for
                    // the current fragment has been updated to check for the following situation:
                    // - Reader notices that the reader's read cursor is the same as the fragment's
                    //   read cursor, indicating that there are no new packets available in the
                    //   current fragment
                    // - Writer (in another thread) writes a packet to the current fragment and
                    //   updates the read cursor
                    // - Writer allocates a new fragment to store another packet
                    // - Reader notices that the next fragment is available and reaches this code
                    // In this situation, checking the read cursor again will allow the reader to
                    // read the remaining packets in the current fragment by allowing the loop to
                    // continue to its next iteration
                    let new_fragment_read_cursor = self.current.read_cursor.load(Ordering::Acquire);
                    if fragment_read_cursor == new_fragment_read_cursor {
                        // There are no more packets on the current fragment.
                        // Start reading from the next fragment
                        self.current = next_fragment;
                        self.read_cursor = 0;
                    }
                } else {
                    // There are no packets at the moment
                    return None;
                }
            } else {
                // Since the fragment's read cursor is only updated when entire packet(s) have been
                // written to the fragment, this code knows that at least 1 packet is ready for
                // reading.

                // Read the packet length
                // SAFETY: This code has not reached the end of the current fragment. An
                // individual packet is guaranteed to be stored contiguously in one fragment.
                // Therefore, a packet size must be valid to read.
                let packet_len: &[u8] = unsafe {
                    self.current
                        .read_unchecked(self.read_cursor..self.read_cursor + size_of::<u32>())
                };
                let packet_len: [u8; size_of::<u32>()] = packet_len.try_into().unwrap();
                let packet_len = u32::from_ne_bytes(packet_len) as usize;
                self.read_cursor += size_of::<u32>();

                // SAFETY: This code has not reached the end of the current fragment. An
                // individual packet is guaranteed to be stored contiguously in one fragment.
                // Therefore, the packet data must be valid to read.
                let packet_data = unsafe {
                    RawPacket::new_unchecked(
                        self.current.clone(),
                        self.read_cursor..self.read_cursor + packet_len,
                    )
                };
                self.read_cursor += packet_len;
                return Some(packet_data);
            }
        }
    }
}

#[derive(Clone)]
pub struct RawPacket {
    fragment: Arc<Fragment>,
    range: Range<usize>,
}

impl RawPacket {
    /// # Safety
    /// `range` must be within `0..read_cursor`
    const unsafe fn new_unchecked(fragment: Arc<Fragment>, range: Range<usize>) -> Self {
        Self { fragment, range }
    }

    #[expect(clippy::missing_const_for_fn, reason = "false positive")]
    #[must_use]
    pub fn fragment_id(&self) -> usize {
        self.fragment.id
    }

    /// Removes the first `n` bytes from this packet
    pub const fn remove_front(&mut self, n: usize) {
        if let Some(range_len) = self.range.end.checked_sub(self.range.start)
            && range_len > n
        {
            // The length of the range is greater than `n`, so adding `n` to the range start is
            // okay and will not cause an integer overflow
            self.range.start += n;
        } else {
            // `n` is greater than or equal to the length of the range, so the resulting range is
            // empty. This code cannot use `start += n` because that could cause an integer
            // overflow
            self.range.start = self.range.end;
        }
    }
}

impl Deref for RawPacket {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: Ensured by safety requirements of RawPacket::new_unchecked
        unsafe { self.fragment.read_unchecked(self.range.clone()) }
    }
}

impl AsRef<[u8]> for RawPacket {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

/// Unbounded spsc channel of `VarInt` length-prefixed packets. The data for any specific packet is stored contiguously.
/// This channel is implemented internally through a linked list of one or more packets in each node, although this is
/// subject to change.
#[must_use]
pub fn channel(default_fragment_size: usize) -> (Sender, Receiver) {
    let fragment = Arc::new(Fragment::new(default_fragment_size, 0));
    let sender = Sender {
        current: fragment.clone(),
        write_cursor: 0,
        default_fragment_size,
        send_state: SendState::ReadLen {
            current_len: 0,
            bit_offset: 0,
        },
    };
    let receiver = Receiver {
        current: fragment,
        read_cursor: 0,
    };
    (sender, receiver)
}

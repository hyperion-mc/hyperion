//! Buffer implementations. todo: We might be able to scrap this for [`bytes::Buf`] in the future.

/// # Safety
/// - `get_contiguous` must return a slice of exactly `len` bytes long.
/// - `advance` must advance the buffer by exactly `len` bytes.
pub unsafe trait Buf {
    /// What type we get when we advance. For example, if we have a [`bytes::BytesMut`], we get a [`bytes::BytesMut`].
    type Output;

    /// Get a contiguous slice of memory of length `len`.
    ///
    /// The returned slice must be exactly `len` bytes long.
    fn get_contiguous(&mut self, len: usize) -> &mut [u8];

    /// Advance the buffer by exactly `len` bytes.
    fn advance(&mut self, len: usize) -> Self::Output;
}

unsafe impl Buf for bytes::BytesMut {
    type Output = Self;

    fn get_contiguous(&mut self, len: usize) -> &mut [u8] {
        // self.resize(len, 0);
        // self
        self.reserve(len);
        let cap = self.spare_capacity_mut();
        unsafe { cap.assume_init_mut() }
    }

    fn advance(&mut self, len: usize) -> Self::Output {
        unsafe { self.set_len(self.len() + len) };
        self.split_to(len)
    }
}

unsafe impl Buf for Vec<u8> {
    type Output = ();

    fn get_contiguous(&mut self, len: usize) -> &mut [u8] {
        // self.resize(len, 0);
        // self
        self.reserve(len);
        let cap = self.spare_capacity_mut();
        unsafe { cap.assume_init_mut() }
    }

    fn advance(&mut self, len: usize) -> Self::Output {
        unsafe { self.set_len(self.len() + len) };
    }
}

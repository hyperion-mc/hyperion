#[cfg(not(feature = "nightly"))]
use std::io::Read;
#[cfg(feature = "nightly")]
use std::{
    alloc::Allocator,
    io::{BorrowedBuf, Read},
};

#[cfg(feature = "nightly")]
#[allow(dead_code, reason = "this might be used in the future")]
pub fn read_to_end<R: Read + ?Sized, A: Allocator>(
    r: &mut R,
    buf: &mut Vec<u8, A>,
) -> std::io::Result<()> {
    const MIN_CAPACITY: usize = 64;

    loop {
        if buf.capacity() == buf.len() {
            let to_reserve = buf.len().max(MIN_CAPACITY);

            buf.reserve(to_reserve);
        }

        let spare = buf.spare_capacity_mut();
        let mut read_buf: BorrowedBuf<'_> = spare.into();
        let mut cursor = read_buf.unfilled();

        r.read_buf(cursor.reborrow())?;

        // let unfilled_but_initialized = cursor.init_ref().len();
        let bytes_read = cursor.written();

        if bytes_read == 0 {
            return Ok(());
        }

        // store how much was initialized but not filled
        // initialized = unfilled_but_initialized;

        // SAFETY: BorrowedBuf's invariants mean this much memory is initialized.
        unsafe {
            let new_len = bytes_read + buf.len();
            buf.set_len(new_len);
        }
    }
}

#[cfg(not(feature = "nightly"))]
#[allow(dead_code, reason = "this might be used in the future")]
pub fn read_to_end<R: Read + ?Sized>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<()> {
    r.read_to_end(buf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use flate2::{Compression, read::ZlibEncoder};

    use crate::net::encoder::util::read_to_end;

    fn rand_slice() -> Vec<u8> {
        let len = fastrand::usize(..8) * fastrand::usize(..64) * fastrand::usize(..64);
        (0..len).map(|_| fastrand::u8(..)).collect()
    }

    fn start_vec_capacity() -> usize {
        fastrand::usize(..8) * fastrand::usize(..64)
    }

    #[test]
    fn test_vec2vec_equivalent_to_std() {
        // seed
        fastrand::seed(7);

        for _ in 0..1_000 {
            let to_read = rand_slice();

            let mut to_read = Cursor::new(to_read);

            let mut buf1 = Vec::new();
            to_read.read_to_end(&mut buf1).unwrap();

            to_read.set_position(0);
            let start_capacity = start_vec_capacity();
            let mut buf2 = Vec::with_capacity(start_capacity);
            read_to_end(&mut to_read, &mut buf2).unwrap();

            assert_eq!(buf1, buf2);
        }
    }

    #[test]
    fn test_zlib_equivalent_to_std() {
        // seed
        fastrand::seed(7);

        let compression = Compression::new(4);

        for _ in 0..1_000 {
            let mut to_read = rand_slice();

            let mut buf1 = Vec::new();
            {
                let to_read = Cursor::new(&mut to_read);
                let mut to_read = ZlibEncoder::new(to_read, compression);

                to_read.read_to_end(&mut buf1).unwrap();
            }

            let start_capacity = start_vec_capacity();
            let mut buf2 = Vec::with_capacity(start_capacity);
            {
                let to_read = Cursor::new(&mut to_read);
                let mut to_read = ZlibEncoder::new(to_read, compression);

                read_to_end(&mut to_read, &mut buf2).unwrap();
            }

            assert_eq!(buf1, buf2);
        }
    }
}

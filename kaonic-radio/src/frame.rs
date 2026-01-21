use core::cmp::min;
use core::fmt;
use core::slice;

use crate::error::KaonicError;

#[derive(Clone, Copy, Debug)]
pub struct FrameSegment<const S: usize, const R: usize> {
    data: [[u8; S]; R],
    len: usize,
}

pub type Frame<const S: usize> = FrameSegment<S, 1>;

impl<const S: usize, const R: usize> FrameSegment<S, R> {
    pub const CAPACITY: usize = S * R;

    pub const fn new() -> Self {
        Self {
            data: [[0u8; S]; R],
            len: 0,
        }
    }

    pub fn new_from_slice(slice: &[u8]) -> Self {
        let mut frame = Self::new();
        frame.copy_from_slice(slice);
        frame
    }

    pub fn capacity(&self) -> usize {
        Self::CAPACITY
    }

    pub fn push_data(&mut self, data: &[u8]) -> Result<usize, KaonicError> {
        let data_size = data.len();
        if self.len + data_size > Self::CAPACITY {
            return Err(KaonicError::OutOfMemory);
        }

        self.alloc_buffer(data_size).copy_from_slice(data);

        Ok(self.len)
    }

    pub fn copy_from_slice(&mut self, data: &[u8]) {
        let len = min(data.len(), Self::CAPACITY);
        self.alloc_buffer(len).copy_from_slice(&data[..len]);
    }

    pub fn as_slice(&self) -> &[u8] {
        let end = self.len;
        &self.as_flat()[..end]
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        let end = self.len;
        &mut self.as_flat_mut()[..end]
    }

    pub fn move_left(&mut self, count: usize) {
        if self.len > count {
            self.as_flat_mut().copy_within(count.., 0);
        }
    }

    pub fn clear(&mut self) -> &mut Self {
        self.len = 0;
        self
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn resize(&mut self, len: usize) {
        self.len = min(len, S);
    }

    pub fn alloc_buffer(&mut self, len: usize) -> &mut [u8] {
        let alloc_len = if self.len + len <= Self::CAPACITY {
            len
        } else {
            Self::CAPACITY - self.len
        };

        let start = self.len;
        self.len += alloc_len;
        let end = self.len;

        &mut self.as_flat_mut()[start..end]
    }

    pub fn alloc_max_buffer(&mut self) -> &mut [u8] {
        self.alloc_buffer(Self::CAPACITY - self.len)
    }

    #[inline]
    fn as_flat(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.data.as_ptr() as *const u8, S * R) }
    }

    #[inline]
    fn as_flat_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.data.as_mut_ptr() as *mut u8, S * R) }
    }
}

impl<const S: usize> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        const BYTES_PER_LINE: usize = 16;

        writeln!(f, "FRAME[{} Bytes]:", self.len)?;

        let data = self.as_flat();
        for (i, chunk) in data[..self.len].chunks(BYTES_PER_LINE).enumerate() {
            // Hex part
            write!(f, "{:08x}  ", i * BYTES_PER_LINE)?;
            for j in 0..BYTES_PER_LINE {
                if j < chunk.len() {
                    write!(f, "{:02x} ", chunk[j])?;
                } else {
                    write!(f, "   ")?; // pad for missing bytes
                }

                // extra space every 8 bytes for readability
                if j == 7 {
                    write!(f, " ")?;
                }
            }

            // ASCII part
            write!(f, " |")?;
            for &b in chunk {
                let c = if (0x20..=0x7e).contains(&b) {
                    b as char
                } else {
                    '.'
                };
                write!(f, "{}", c)?;
            }
            writeln!(f, "|")?;
        }

        Ok(())
    }
}

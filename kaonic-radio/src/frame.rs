use core::cmp::min;
use core::fmt;

use crate::error::KaonicError;

#[derive(Clone, Copy, Debug)]
pub struct Frame<const S: usize> {
    data: [u8; S],
    len: usize,
}

impl<const S: usize> Frame<S> {
    pub const fn new() -> Self {
        Self {
            data: [0u8; S],
            len: 0,
        }
    }

    pub fn new_from_slice(slice: &[u8]) -> Self {
        let len = core::cmp::min(slice.len(), S);
        let mut data = [0u8; S];

        data[..len].copy_from_slice(&slice[..len]);

        Self { data, len }
    }

    pub fn capacity(&self) -> usize {
        S
    }

    pub fn push_data(&mut self, data: &[u8]) -> Result<usize, KaonicError> {
        let data_size = data.len();
        if self.len + data_size > S {
            return Err(KaonicError::OutOfMemory);
        }

        self.data[self.len..(self.len + data_size)].copy_from_slice(data);
        self.len += data_size;

        Ok(self.len)
    }

    pub fn copy_from_slice(&mut self, data: &[u8]) {
        self.len = min(data.len(), S);
        self.data[..self.len].copy_from_slice(&data[..self.len]);
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn move_left(&mut self, count: usize) {
        if self.len > count {
            self.data.copy_within(count.., 0);
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn resize(&mut self, len: usize) {
        self.len = min(len, S);
    }

    pub fn as_buffer_mut(&mut self, len: usize) -> &mut [u8] {
        let alloc_len = if self.len + len <= S {
            len
        } else {
            S - self.len
        };

        let buffer = &mut self.data[self.len..self.len + alloc_len];
        self.len += alloc_len;

        buffer
    }

    pub fn as_max_buffer_mut(&mut self) -> &mut [u8] {
        self.as_buffer_mut(S - self.len)
    }
}

impl<const S: usize> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        const BYTES_PER_LINE: usize = 16;

        writeln!(f, "FRAME[{} Bytes]:", self.len)?;

        for (i, chunk) in self.data[..self.len].chunks(BYTES_PER_LINE).enumerate() {
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

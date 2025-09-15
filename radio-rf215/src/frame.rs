use core::fmt;

pub struct Frame<const S: usize> {
    data: [u8; S],
    len: usize,
}

impl<const S: usize> Frame<S> {
    pub fn new() -> Self {
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

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_buffer_mut(&mut self, len: usize) -> &mut [u8] {
        self.len = if len <= S { len } else { S };
        &mut self.data[..self.len]
    }
}

impl<const S: usize> fmt::Display for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut offset = 0;

        write!(f, "FRAME[{} Bytes]:\n\r", self.len)?;

        for i in 0..self.len {
            write!(f, " {:0>2x}", self.data[i])?;

            offset += 1;
            if offset == 16 {
                write!(f, "|\n\r")?;
                offset = 0;
            }
        }

        Ok(())
    }
}

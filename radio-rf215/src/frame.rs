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

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn as_buffer_mut(&mut self, len: usize) -> &mut [u8] {
        self.len = if len <= S { len } else { S };
        &mut self.data[..self.len]
    }
}

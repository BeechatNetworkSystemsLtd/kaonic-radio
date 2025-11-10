use crate::frame::Frame;

#[derive(Clone, Copy, Debug)]
pub struct Payload<const S: usize> {
    frame: Frame<S>,
}

impl<const S: usize> Payload<S> {
    pub const fn new() -> Self {
        Self {
            frame: Frame::<S>::new(),
        }
    }

    pub fn build(self) -> Frame<S> {
        let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        let mut digest = crc.digest();

        let mut frame = self.frame;
        let len = frame.len();
        let max_payload_len = core::cmp::min(S - core::mem::size_of::<u32>(), len);
        frame.resize(max_payload_len);
        digest.update(frame.as_slice());
        frame.push_data(&digest.finalize().to_le_bytes()[..]);

        frame
    }
}

use kaonic_radio::{
    error::KaonicError,
    frame::{Frame, FrameSegment},
};
use rand::{CryptoRng, RngCore};

use crate::{
    demuxer::Demuxer,
    generator::Generator,
    muxer::{CurrentTime, Muxer},
    packet::{Packet, PacketCoder},
};

pub struct Network<
    const S: usize,
    const R: usize,
    const Q: usize,
    const P: usize,
    C: PacketCoder<S>,
> {
    demuxer: Demuxer<S, R, P>,
    muxer: Muxer<S, R, Q>,
    packets: [Packet<S>; R],
    input_frame: FrameSegment<S, R>,
    coder: C,
}

impl<const S: usize, const R: usize, const Q: usize, const P: usize, C: PacketCoder<S>>
    Network<S, R, Q, P, C>
{
    pub fn new(coder: C) -> Self {
        Self {
            demuxer: Demuxer::new(),
            muxer: Muxer::new(),
            packets: [Packet::new(); R],
            input_frame: FrameSegment::new(),
            coder,
        }
    }

    pub fn receive(
        &mut self,
        current_time: CurrentTime,
        frame: &Frame<S>,
    ) -> Result<(), KaonicError> {
        self.coder.decode(&frame, &mut self.packets[0])?;

        let _ = self.muxer.multiplex(current_time, &self.packets[0]);

        Ok(())
    }

    pub fn process<F>(&mut self, current_time: CurrentTime, receive_func: F)
    where
        F: FnOnce(&[u8]),
    {
        if let Ok(frame) = self.muxer.process(current_time, &mut self.input_frame) {
            receive_func(frame.as_slice());
        }
    }

    pub fn transmit<'a, RNG: CryptoRng + RngCore + Copy, F>(
        &mut self,
        data: &[u8],
        rng: RNG,
        output_frames: &'a mut [Frame<S>],
        transmit_func: F,
    ) -> Result<(), KaonicError>
    where
        F: FnOnce(&[&[u8]]) -> Result<(), KaonicError>,
    {
        let packet_id = Generator::generate_packet_id(rng)?;

        let packets = self
            .demuxer
            .demultiplex(packet_id, data, &mut self.packets[..])?;

        if output_frames.len() < packets.len() {
            return Err(KaonicError::PayloadTooBig);
        }

        let mut frames_data: [&[u8]; R] = [&[]; R];

        for i in 0..packets.len() {
            self.coder.encode(&packets[i], &mut output_frames[i])?;
        }

        for i in 0..packets.len() {
            frames_data[i] = output_frames[i].as_slice();
        }

        transmit_func(&frames_data[..packets.len()])
    }
}

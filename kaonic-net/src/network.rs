use kaonic_frame::frame::{Frame, FrameSegment};
use rand::{CryptoRng, RngCore};

use crate::{
    coder::PacketCoder,
    demuxer::Demuxer,
    error::NetworkError,
    generator::Generator,
    muxer::Muxer,
    packet::{AssembledPacket, Packet},
    NetworkTime,
};

/// Network packet processing pipeline.
///
/// Const generic parameters:
/// - `S`: Frame payload size in bytes for each [`Frame`].
/// - `R`: Maximum number of packet fragments/reassembly slots handled at once.
/// - `Q`: Maximum number of packets tracked in the mux queue.
pub struct Network<
    const S: usize,
    const R: usize,
    const Q: usize,
    C: PacketCoder<S>,
> {
    demuxer: Demuxer<S, R>,
    muxer: Muxer<S, R, Q>,
    packets: [Packet<S>; R],
    coder: C,
}

impl<const S: usize, const R: usize, const Q: usize, C: PacketCoder<S>>
    Network<S, R, Q, C>
{
    pub fn new(coder: C) -> Self {
        Self {
            demuxer: Demuxer::new(C::MAX_PAYLOAD_SIZE),
            muxer: Muxer::new(),
            packets: [Packet::new(); R],
            coder,
        }
    }

    pub fn receive(
        &mut self,
        current_time: NetworkTime,
        frame: &Frame<S>,
    ) -> Result<(), NetworkError> {
        self.coder.decode(&frame, &mut self.packets[0])?;

        let _ = self.muxer.multiplex(current_time, &self.packets[0]);

        Ok(())
    }

    pub fn process<'a>(
        &mut self,
        current_time: NetworkTime,
        rx_frame: &'a mut FrameSegment<S, R>,
    ) -> Result<AssembledPacket<'a, S, R>, NetworkError> {
        let packet = self.muxer.process(rx_frame);

        self.muxer.release_expired(current_time);

        packet
    }

    pub fn transmit<'a, RNG: CryptoRng + RngCore + Copy>(
        &mut self,
        data: &[u8],
        rng: RNG,
        output_frames: &'a mut [Frame<S>],
    ) -> Result<&'a [Frame<S>], NetworkError> {
        let packet_id = Generator::generate_packet_id(rng)?;

        let packets = self
            .demuxer
            .demultiplex(packet_id, data, &mut self.packets[..])?;

        if output_frames.len() < packets.len() {
            return Err(NetworkError::PayloadTooBig);
        }

        for i in 0..packets.len() {
            self.coder.encode(&packets[i], &mut output_frames[i])?;
        }

        Ok(&output_frames[..packets.len()])
    }
}

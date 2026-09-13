use kaonic_frame::frame::{Frame, FrameSegment};
use rand::{CryptoRng, RngCore};

use crate::{
    coder::PacketCoder,
    demuxer::Demuxer,
    error::NetworkError,
    generator::Generator,
    muxer::{Multiplexed, Muxer},
    packet::{AssembledPacket, Packet},
    NetworkTime,
};

/// What [`Network::receive`] did with a frame that decoded. Only `Slotted`
/// is the ordinary case; the rest are loss or waste that would otherwise be
/// invisible, since the decode itself succeeded.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Received {
    Slotted,
    /// Stored, at the cost of the oldest partial payload.
    Evicted,
    /// A segment already held; nothing stored.
    Duplicate,
    /// No slot could ever hold it (too many segments, or not segmented).
    Refused,
}

/// Network packet processing pipeline.
///
/// Const generic parameters:
/// - `S`: Frame payload size in bytes for each [`Frame`].
/// - `R`: Maximum number of packet fragments/reassembly slots handled at once.
/// - `Q`: Maximum number of packets tracked in the mux queue.

#[derive(Debug)]
pub struct Network<const S: usize, const R: usize, const Q: usize, C: PacketCoder<S>> {
    demuxer: Demuxer<S, R>,
    muxer: Muxer<S, R, Q>,
    packets: [Packet<S>; R],
    coder: C,
}

impl<const S: usize, const R: usize, const Q: usize, C: PacketCoder<S>> Network<S, R, Q, C> {
    pub fn new(coder: C) -> Self {
        Self::new_with(coder, Muxer::<S, R, Q>::DEFAULT_TIMEOUT)
    }

    /// `reassembly_window` is how long a partial payload waits for the rest of
    /// its segments before its slot is given up.
    pub fn new_with(coder: C, reassembly_window: core::time::Duration) -> Self {
        Self::new_sized(coder, reassembly_window, C::MAX_PAYLOAD_SIZE)
    }

    /// `segment` is the payload each frame carries. The default is what the
    /// strongest code fits, which is safe for any code but leaves a lighter
    /// code's frames a third empty; a channel whose code is fixed can cut its
    /// frames to that code's real capacity.
    pub fn new_sized(coder: C, reassembly_window: core::time::Duration, segment: usize) -> Self {
        Self {
            demuxer: Demuxer::new(segment),
            muxer: Muxer::with_timeout(reassembly_window),
            packets: [Packet::new(); R],
            coder,
        }
    }

    /// Reassembly slots currently holding a payload.
    pub fn slots_in_use(&self) -> usize {
        self.muxer.in_use()
    }

    pub fn coder(&self) -> &C {
        &self.coder
    }

    pub fn coder_mut(&mut self) -> &mut C {
        &mut self.coder
    }

    pub fn receive(
        &mut self,
        current_time: NetworkTime,
        frame: &Frame<S>,
    ) -> Result<Received, NetworkError> {
        self.coder.decode(&frame, &mut self.packets[0])?;

        // A frame that decoded but found no home is not a decode failure, and
        // the caller must be able to tell the two apart: one is the air, the
        // other is this table running out of room.
        Ok(match self.muxer.multiplex(current_time, &self.packets[0]) {
            Ok(Multiplexed::Slotted) => Received::Slotted,
            Ok(Multiplexed::Evicted) => Received::Evicted,
            Ok(Multiplexed::Duplicate) => Received::Duplicate,
            Err(_) => Received::Refused,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coder::{FecCode, LdpcPacketCoder};
    use kaonic_frame::frame::Frame;
    use rand::rngs::OsRng;

    const S: usize = 2048;
    /// What a bonded channel uses: six frames per payload rather than three.
    const SEG: usize = 6;

    type Net = Network<S, SEG, 4, LdpcPacketCoder<S>>;

    /// A bonded channel spreads one payload's frames over several radios, so
    /// the receiver sees them interleaved and out of order. Reassembly keys on
    /// the sequence number in the header, never on which radio a frame came
    /// from, so this has to hold for bonding to work at all.
    #[test]
    fn a_six_segment_payload_reassembles_out_of_order() {
        let capacity = LdpcPacketCoder::<S>::max_payload_for(FecCode::Tm2048);
        // Longer than three segments: unreachable on a single-module channel.
        let payload: Vec<u8> = (0..capacity * SEG)
            .map(|i| (i % 251) as u8)
            .collect();

        let mut tx = Net::new(LdpcPacketCoder::new());
        let mut frames = [Frame::<S>::new(); SEG];
        let sent: Vec<Frame<S>> = tx
            .transmit(&payload, OsRng, &mut frames)
            .expect("six segments fit")
            .to_vec();
        assert_eq!(sent.len(), SEG, "payload should span every segment");

        // Deliver as two radios would: all of radio 1's frames (the odd
        // sequence numbers) before any of radio 0's.
        let mut rx = Net::new(LdpcPacketCoder::new());
        let order = [1usize, 3, 5, 0, 2, 4];
        for i in order {
            rx.receive(0, &sent[i]).expect("frame decodes");
        }

        let mut assembly = FrameSegment::<S, SEG>::new();
        let packet = rx.process(0, &mut assembly).expect("packet assembles");
        assert_eq!(packet.as_slice(), &payload[..]);
    }

    /// The single-module MTU still holds where it did, so an unbonded channel
    /// stays byte-identical to what nodes on the air expect.
    #[test]
    fn a_payload_longer_than_the_segment_budget_is_refused() {
        let capacity = LdpcPacketCoder::<S>::max_payload_for(FecCode::Tm2048);
        let mut tx = Network::<S, 3, 4, LdpcPacketCoder<S>>::new(LdpcPacketCoder::new());
        let mut frames = [Frame::<S>::new(); 3];
        let payload = vec![0u8; capacity * 3 + 1];
        assert!(tx.transmit(&payload, OsRng, &mut frames).is_err());
    }
}

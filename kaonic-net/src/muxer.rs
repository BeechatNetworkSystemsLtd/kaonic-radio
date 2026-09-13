use kaonic_frame::frame::FrameSegment;

use crate::{
    error::NetworkError,
    network_time_elapsed,
    packet::{AssembledPacket, Packet, PacketFlag, PacketId},
    NetworkTime,
};

#[derive(Copy, Clone, Debug)]
pub struct PacketMuxer<const S: usize, const R: usize> {
    packets: [Packet<S>; R],
    count: usize,
    last_update_time: NetworkTime,
}

impl<const S: usize, const R: usize> PacketMuxer<S, R> {
    pub fn new() -> Self {
        Self {
            count: 0,
            packets: [Packet::new(); R],
            last_update_time: 0,
        }
    }

    pub fn packet_id(&self) -> PacketId {
        if self.count == 0 {
            return 0;
        } else {
            self.packets[0].header().id()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn push(&mut self, current_time: NetworkTime, new_packet: &Packet<S>) -> bool {
        // Packet collection is already full
        if self.count >= R {
            return false;
        }

        let new_header = new_packet.header();

        // Expected sequence count is greater than supported
        if new_header.seq_count() > R {
            return false;
        }

        if self.count != 0 {
            // Check if new packet can be added to the collection
            for i in 0..self.count {
                let packet = &self.packets[i];
                let header = packet.header();

                // Packet id doesn't match
                if header.id() != new_header.id() {
                    return false;
                }

                // This sequance packet is already exist
                if header.seq() == new_header.seq() {
                    return false;
                }
            }
        }

        // Add new packet to the collection
        self.packets[self.count] = *new_packet;
        self.count += 1;

        self.last_update_time = current_time;

        true
    }

    pub fn release(&mut self) {
        self.count = 0;
        self.last_update_time = 0;
    }

    pub fn timeout_reached(
        &self,
        current_time: NetworkTime,
        timeout: core::time::Duration,
    ) -> bool {
        network_time_elapsed(self.last_update_time, current_time, timeout)
    }

    pub fn can_assemble(&self) -> bool {
        if self.count == 0 {
            return false;
        }

        let header = self.packets[0].header();

        if self.count < header.seq_count() {
            return false;
        }

        return true;
    }

    pub fn assemble<'a>(
        &self,
        frame: &'a mut FrameSegment<S, R>,
    ) -> Result<AssembledPacket<'a, S, R>, NetworkError> {
        if !self.can_assemble() {
            return Err(NetworkError::TryAgain);
        }

        // Use first packet as common header
        let header = self.packets[0].header();

        frame.clear();

        let mut seq = 0;
        let mut iter = 0;

        // Repeat 'count' iterations
        while iter < self.count {
            let mut seq_found = false;
            for packet in &self.packets {
                if seq == packet.header().seq() {
                    frame.push_data(packet.frame().as_slice())?;
                    seq += 1;
                    seq_found = true;
                    break;
                }
            }

            // Sequance was not found in the collection
            if !seq_found {
                return Err(NetworkError::IncorrectSequence);
            }

            iter += 1;
        }

        Ok(AssembledPacket::new(header.id(), frame))
    }
}

/// What became of a segment handed to [`Muxer::multiplex`]. A caller that
/// counts loss needs the difference: an eviction is a payload that will never
/// complete, a duplicate is a frame the air delivered twice, and neither is a
/// decode failure.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Multiplexed {
    /// Joined an existing slot or took a free one.
    Slotted,
    /// Took a slot, and the oldest partial payload was thrown away for it.
    Evicted,
    /// A sequence number already held for this packet id; nothing stored.
    Duplicate,
}

/// The muxer can handle up to 'Q' packets divided into 'R' segments of 'S' size
#[derive(Debug)]
pub struct Muxer<const S: usize, const R: usize, const Q: usize> {
    queue: [PacketMuxer<S, R>; Q],
    timeout: core::time::Duration,
}

impl<const S: usize, const R: usize, const Q: usize> Muxer<S, R, Q> {
    pub const DEFAULT_TIMEOUT: core::time::Duration = core::time::Duration::from_millis(500);

    pub fn new() -> Self {
        Self::with_timeout(Self::DEFAULT_TIMEOUT)
    }

    /// `timeout` is how long a partial payload waits for its remaining
    /// segments. It belongs to the link, not the muxer: over a slow radio the
    /// sibling frames of a bonded payload can be hundreds of milliseconds
    /// apart, and a fixed half second either drops them or hoards slots.
    pub fn with_timeout(timeout: core::time::Duration) -> Self {
        Self {
            queue: [PacketMuxer::new(); Q],
            timeout,
        }
    }

    /// Slots holding a partial or complete payload.
    pub fn in_use(&self) -> usize {
        self.queue.iter().filter(|px| !px.is_empty()).count()
    }

    pub fn multiplex(
        &mut self,
        current_time: NetworkTime,
        packet: &Packet<S>,
    ) -> Result<Multiplexed, NetworkError> {
        if !packet.header().has_flag(PacketFlag::Segmented) {
            return Err(NetworkError::NotSupported);
        }

        // More segments than a slot can hold: nothing here could ever finish
        // it, so it is refused before it costs anyone a slot.
        if packet.header().seq_count() > R {
            return Err(NetworkError::PayloadTooBig);
        }

        let packet_id = packet.header().id();

        // A slot already holding this id is the only place the segment may
        // go. When it refuses the segment, that is a repeat of a sequence
        // number it has, and the repeat must not start a second slot: the
        // copy would never complete and would only push a live payload out.
        if let Some(px) = self
            .queue
            .iter_mut()
            .find(|px| !px.is_empty() && px.packet_id() == packet_id)
        {
            return if px.push(current_time, packet) {
                Ok(Multiplexed::Slotted)
            } else {
                Ok(Multiplexed::Duplicate)
            };
        }

        for px in self.queue.iter_mut() {
            if px.timeout_reached(current_time, self.timeout) {
                px.release();
            }

            if px.is_empty() {
                if px.push(current_time, packet) {
                    return Ok(Multiplexed::Slotted);
                }
            }
        }

        // Every slot is busy. The newcomer is a payload that is still being
        // sent, while the slot that has waited longest is the one least likely
        // to finish; dropping the newcomer instead would lose payloads
        // precisely while the air is at its busiest.
        // A slot that is complete and only waiting to be assembled is not a
        // candidate while a partial one exists.
        let oldest = self
            .queue
            .iter_mut()
            .min_by_key(|px| (px.can_assemble(), px.last_update_time))
            .expect("Q > 0");
        oldest.release();
        if oldest.push(current_time, packet) {
            Ok(Multiplexed::Evicted)
        } else {
            Err(NetworkError::TryAgain)
        }
    }

    pub fn process<'a>(
        &mut self,
        frame: &'a mut FrameSegment<S, R>,
    ) -> Result<AssembledPacket<'a, S, R>, NetworkError> {
        // Find one assembled packet
        let px = self.queue.iter_mut().find(|px| px.can_assemble());

        if let Some(px) = px {
            if let Ok(packet) = px.assemble(frame) {
                px.release();
                return Ok(packet);
            }
        }

        return Err(NetworkError::TryAgain);
    }

    pub fn release_expired(&mut self, current_time: NetworkTime) {
        // Release all expired packets
        for px in self.queue.iter_mut() {
            if px.timeout_reached(current_time, self.timeout) {
                px.release();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    const S: usize = 64;
    const R: usize = 3;

    /// One segment of a three-segment payload, as the demuxer would build it.
    fn segment(id: PacketId, seq: usize) -> Packet<S> {
        let mut packet = Packet::new();
        packet
            .header_mut()
            .reset()
            .add_flag(PacketFlag::Segmented)
            .set_id(id)
            .set_seq(seq)
            .set_seq_count(R);
        packet.frame_mut().push_data(&[seq as u8; 8]).unwrap();
        packet.build();
        packet
    }

    /// The air can deliver a frame twice (a retransmit, or the same frame
    /// heard on both radios). The repeat must land on the slot that already
    /// holds the id, never on a fresh one.
    #[test]
    fn a_duplicate_segment_does_not_open_a_second_slot() {
        let mut muxer = Muxer::<S, R, 2>::new();

        assert_eq!(muxer.multiplex(0, &segment(7, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.multiplex(1, &segment(7, 0)), Ok(Multiplexed::Duplicate));
        assert_eq!(muxer.in_use(), 1, "the repeat took no slot");

        // The second slot is still free for another payload, and only a third
        // one has to evict.
        assert_eq!(muxer.multiplex(2, &segment(8, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.multiplex(3, &segment(9, 0)), Ok(Multiplexed::Evicted));
    }

    /// With every slot busy the payload that has waited longest gives way,
    /// and the newcomer — the one actually being sent right now — is kept.
    #[test]
    fn the_oldest_partial_is_evicted_when_every_slot_is_busy() {
        let mut muxer = Muxer::<S, R, 2>::new();

        assert_eq!(muxer.multiplex(0, &segment(1, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.multiplex(10, &segment(2, 0)), Ok(Multiplexed::Slotted));
        // Touching id 1 makes id 2 the one that has waited longest.
        assert_eq!(muxer.multiplex(20, &segment(1, 1)), Ok(Multiplexed::Slotted));

        assert_eq!(muxer.multiplex(30, &segment(3, 0)), Ok(Multiplexed::Evicted));
        assert_eq!(muxer.in_use(), 2);

        // Id 1 survived and still completes; id 2 is gone and starts over.
        let mut assembly = FrameSegment::<S, R>::new();
        assert_eq!(muxer.multiplex(40, &segment(1, 2)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.process(&mut assembly).unwrap().id(), 1);
        assert_eq!(muxer.multiplex(50, &segment(2, 1)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.in_use(), 2);
    }

    /// A complete payload waiting to be handed out is never the one evicted
    /// while a partial one exists.
    #[test]
    fn a_complete_payload_is_not_evicted_before_a_partial_one() {
        let mut muxer = Muxer::<S, R, 2>::new();

        for seq in 0..R {
            assert_eq!(muxer.multiplex(0, &segment(1, seq)), Ok(Multiplexed::Slotted));
        }
        assert_eq!(muxer.multiplex(10, &segment(2, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(muxer.multiplex(20, &segment(3, 0)), Ok(Multiplexed::Evicted));

        let mut assembly = FrameSegment::<S, R>::new();
        assert_eq!(muxer.process(&mut assembly).unwrap().id(), 1);
    }

    /// The window is the link's, not a constant: a short one frees a stale
    /// partial before a newcomer has to evict anything, the default does not.
    #[test]
    fn the_reassembly_timeout_is_the_one_configured() {
        let mut short = Muxer::<S, R, 1>::with_timeout(Duration::from_millis(100));
        assert_eq!(short.multiplex(0, &segment(1, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(short.multiplex(150, &segment(2, 0)), Ok(Multiplexed::Slotted));

        let mut default = Muxer::<S, R, 1>::new();
        assert_eq!(default.multiplex(0, &segment(1, 0)), Ok(Multiplexed::Slotted));
        assert_eq!(default.multiplex(150, &segment(2, 0)), Ok(Multiplexed::Evicted));
        assert_eq!(default.multiplex(700, &segment(3, 0)), Ok(Multiplexed::Slotted));
    }

    /// More segments than a slot holds can never complete here, so the
    /// segment is refused rather than allowed to hold or evict anything.
    #[test]
    fn an_oversized_payload_is_refused_without_taking_a_slot() {
        let mut muxer = Muxer::<S, R, 1>::new();
        let mut packet = segment(1, 0);
        packet.header_mut().set_seq_count(R + 1);
        assert!(muxer.multiplex(0, &packet).is_err());
        assert_eq!(muxer.in_use(), 0);
    }
}

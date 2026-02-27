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

/// The muxer can handle up to 'Q' packets divided into 'R' segments of 'S' size
#[derive(Debug)]
pub struct Muxer<const S: usize, const R: usize, const Q: usize> {
    queue: [PacketMuxer<S, R>; Q],
    timeout: core::time::Duration,
}

impl<const S: usize, const R: usize, const Q: usize> Muxer<S, R, Q> {
    pub fn new() -> Self {
        Self {
            queue: [PacketMuxer::new(); Q],
            timeout: core::time::Duration::from_millis(500),
        }
    }

    pub fn multiplex(
        &mut self,
        current_time: NetworkTime,
        packet: &Packet<S>,
    ) -> Result<(), NetworkError> {
        if !packet.header().has_flag(PacketFlag::Segmented) {
            return Err(NetworkError::NotSupported);
        }

        let packet_id = packet.header().id();

        for px in self.queue.iter_mut() {
            if !px.is_empty() && px.packet_id() == packet_id {
                if px.push(current_time, packet) {
                    return Ok(());
                }
            }
        }

        for px in self.queue.iter_mut() {
            if px.timeout_reached(current_time, self.timeout) {
                px.release();
            }

            if px.is_empty() {
                if px.push(current_time, packet) {
                    return Ok(());
                }
            }
        }

        Err(NetworkError::TryAgain)
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

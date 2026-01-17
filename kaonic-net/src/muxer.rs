use kaonic_radio::{error::KaonicError, frame::FrameSegment};

use crate::packet::{self, Packet, PacketFlag, PacketId};

pub type CurrentTime = u128;

#[derive(Copy, Clone, Debug)]
pub struct PacketMuxer<const S: usize, const R: usize> {
    packets: [Packet<S>; R],
    count: usize,
    last_update_time: CurrentTime,
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

    pub fn push(&mut self, current_time: CurrentTime, new_packet: &Packet<S>) -> bool {
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
        current_time: CurrentTime,
        timeout: core::time::Duration,
    ) -> bool {
        let timeout = current_time + timeout.as_millis();
        current_time > timeout
    }

    pub fn assemble(&self, frame: &mut FrameSegment<S, R>) -> Result<(), KaonicError> {
        // No packets in the collection for assembly
        if self.count == 0 {
            return Err(KaonicError::TryAgain);
        }

        // Use first packet as common header
        let header = self.packets[0].header();

        if self.count < header.seq_count() {
            return Err(KaonicError::TryAgain);
        }

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
                return Err(KaonicError::InvalidState);
            }

            iter += 1;
        }

        Ok(())
    }
}

/// The muxer can handle up to 'Q' packets divided into 'R' segments of 'S' size
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
        current_time: CurrentTime,
        packet: &Packet<S>,
    ) -> Result<(), KaonicError> {
        if !packet.header().has_flag(PacketFlag::Segmented) {
            return Err(KaonicError::NotSupported);
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
            if px.is_empty() {
                if px.push(current_time, packet) {
                    return Ok(());
                }
            }
        }

        Err(KaonicError::TryAgain)
    }

    pub fn process<'a>(
        &mut self,
        current_time: CurrentTime,
        frame: &'a mut FrameSegment<S, R>,
    ) -> Result<&'a FrameSegment<S, R>, KaonicError> {
        let mut result: Result<&'a FrameSegment<S, R>, KaonicError> = Err(KaonicError::TryAgain);

        // Find one assembled packet
        for px in self.queue.iter_mut() {
            match px.assemble(frame) {
                Ok(_) => {
                    px.release();
                    result = Ok(frame);
                    break;
                }

                Err(_) => {}
            }
        }

        // Release all expired packets
        for px in self.queue.iter_mut() {
            if px.timeout_reached(current_time, self.timeout) {
                px.release();
            }
        }

        result
    }
}

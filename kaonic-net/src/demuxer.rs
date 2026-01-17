use core::{
    cmp::PartialEq,
    ops::{Add, Div, Rem},
};

use kaonic_radio::error::KaonicError;

use crate::packet::{Packet, PacketFlag, PacketId};

pub struct Demuxer<const S: usize, const R: usize, const P: usize> {}

impl<const S: usize, const R: usize, const P: usize> Demuxer<S, R, P> {
    pub const MAX_PACKET_PAYLOAD_SIZE: usize = P;
    pub const MAX_PAYLOAD_SIZE: usize = P * R;

    pub fn new() -> Self {
        Self {}
    }

    pub fn max_payload_size(&self) -> usize {
        Self::MAX_PAYLOAD_SIZE
    }

    pub fn max_packet_payload_size(&self) -> usize {
        Self::MAX_PACKET_PAYLOAD_SIZE
    }

    pub fn demultiplex<'a>(
        &mut self,
        id: PacketId,
        payload_data: &[u8],
        packets: &'a mut [Packet<S>],
    ) -> Result<&'a [Packet<S>], KaonicError> {
        // Check if payload can fit into overall demux process
        let total_len = payload_data.len();
        if total_len > Self::MAX_PAYLOAD_SIZE {
            return Err(KaonicError::PayloadTooBig);
        }

        let segment_size = Self::MAX_PACKET_PAYLOAD_SIZE;
        if segment_size > (u16::max_value() as usize) {
            return Err(KaonicError::PayloadTooBig);
        }

        let seq_count = div_round_up(total_len, segment_size);
        if seq_count > packets.len() {
            return Err(KaonicError::PayloadTooBig);
        }

        let mut seq = 0;
        let mut offset = 0usize;
        while offset < total_len {
            if seq > packets.len() {
                return Err(KaonicError::PayloadTooBig);
            }

            let chunk_len = if offset + segment_size < total_len {
                segment_size
            } else {
                total_len - offset
            };

            let chunk = &payload_data[offset..(offset + chunk_len)];

            offset += chunk_len;

            let packet = &mut packets[seq];
            {
                packet.reset();

                let header = packet.header_mut();
                {
                    header
                        .reset()
                        .add_flag(PacketFlag::Encoded)
                        .add_flag(PacketFlag::Segmented)
                        .set_id(id)
                        .set_seq(seq)
                        .set_seq_count(seq_count)
                        .set_len(chunk.len() as u16);
                }

                packet.frame_mut().push_data(chunk)?;

                packet.build();
            }

            seq += 1;
        }

        Ok(&packets[..seq])
    }
}

#[inline]
fn div_round_up<T>(n: T, d: T) -> T
where
    T: Copy + Div<Output = T> + Rem<Output = T> + From<u8> + Add<Output = T> + PartialEq,
{
    n / d + T::from(if n % d != T::from(0) { 1 } else { 0 })
}

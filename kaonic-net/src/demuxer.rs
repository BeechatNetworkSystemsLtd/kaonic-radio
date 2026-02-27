use core::{
    cmp::PartialEq,
    ops::{Add, Div, Rem},
};

use crate::{
    error::NetworkError,
    packet::{Packet, PacketFlag, PacketId},
};

/// Splits a payload into packet-sized segments.
///
/// Const generic parameters:
/// - `S`: Frame payload size in bytes for each [`Packet`].
/// - `R`: Maximum number of packet segments handled per demultiplex operation.
/// - `P`: Maximum payload size in bytes per packet segment.
#[derive(Debug)]
pub struct Demuxer<const S: usize, const R: usize> {
    total_size: usize,
    segment_size: usize,
}

impl<const S: usize, const R: usize> Demuxer<S, R> {
    pub fn new(segment_size: usize) -> Self {
        Self {
            segment_size,
            total_size: segment_size * R,
        }
    }
    pub fn demultiplex<'a>(
        &mut self,
        id: PacketId,
        payload_data: &[u8],
        packets: &'a mut [Packet<S>],
    ) -> Result<&'a [Packet<S>], NetworkError> {
        // Check if payload can fit into overall demux process
        let total_len = payload_data.len();
        if total_len > self.total_size {
            return Err(NetworkError::PayloadTooBig);
        }

        let segment_size = self.segment_size;
        if segment_size > (u16::max_value() as usize) {
            return Err(NetworkError::PayloadTooBig);
        }

        let seq_count = div_round_up(total_len, segment_size);
        if seq_count > packets.len() {
            return Err(NetworkError::PayloadTooBig);
        }

        let mut seq = 0;
        let mut offset = 0usize;
        while offset < total_len {
            if seq > packets.len() {
                return Err(NetworkError::PayloadTooBig);
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

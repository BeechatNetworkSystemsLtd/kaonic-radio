use core::fmt;

use labrador_ldpc::LDPCCode;

use kaonic_radio::{error::KaonicError, frame::Frame};

pub const HEADER_SIZE: usize = 16;

pub const HEADER_LDPC_CODE: LDPCCode = LDPCCode::TC256;
pub const PAYLOAD_LDPC_CODE: LDPCCode = LDPCCode::TM2048;

pub const PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE: usize = PAYLOAD_LDPC_CODE.output_len();
pub const PAYLOAD_LDPC_WORKING_BUFFER_SIZE: usize = PAYLOAD_LDPC_CODE.decode_bf_working_len();

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PacketType {
    Payload = 0xBA,
}

pub type PacketId = u32;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PacketFlag {
    /// Header and payload are encoded with correction codes
    Encoded = 0x01,
    /// Large payload is split into segments
    Segmented = 0x02,
}

#[derive(Copy, Clone, Debug)]
pub struct Header {
    /// Type of the packet
    packet_type: PacketType,

    /// Packet identifier
    id: PacketId,

    /// Bitmap of PacketFlag
    flags: u8,

    /// Packet Sequence number
    seq: usize,

    /// Number of packets
    seq_count: usize,

    /// Packet payload length
    len: u16,

    // CRC
    crc: u32,
}

impl Header {
    pub const fn new() -> Self {
        Self {
            packet_type: PacketType::Payload,
            id: 0,
            flags: 0,
            seq: 0,
            seq_count: 0,
            len: 0,
            crc: 0,
        }
    }

    pub fn reset(&mut self) -> &mut Self {
        *self = Header::new();
        self
    }

    pub fn set_id(&mut self, id: PacketId) -> &mut Self {
        self.id = id;
        self
    }

    pub fn id(&self) -> PacketId {
        self.id
    }

    pub fn set_seq(&mut self, seq: usize) -> &mut Self {
        self.seq = seq;
        self
    }

    pub fn seq(&self) -> usize {
        self.seq
    }

    pub fn set_seq_count(&mut self, seq_count: usize) -> &mut Self {
        self.seq_count = seq_count;
        self
    }

    pub fn seq_count(&self) -> usize {
        self.seq_count
    }

    pub fn add_flag(&mut self, flag: PacketFlag) -> &mut Self {
        self.flags = self.flags | (flag as u8);
        self
    }

    pub fn remove_flag(&mut self, flag: PacketFlag) -> &mut Self {
        self.flags = self.flags & (!(flag as u8));
        self
    }

    pub fn has_flag(&self, flag: PacketFlag) -> bool {
        (self.flags & (flag as u8)) != 0u8
    }

    pub fn set_len(&mut self, len: u16) -> &mut Self {
        self.len = len;
        self
    }

    pub fn len(&self) -> u16 {
        self.len
    }

    pub fn crc(&self) -> u32 {
        self.crc
    }

    fn pack(&self) -> [u8; HEADER_SIZE] {
        let mut buffer: [u8; HEADER_SIZE] = [0; HEADER_SIZE];

        let mut offset = 0usize;

        buffer[offset] = self.packet_type as u8;
        offset += 1;

        buffer[offset] = self.flags;
        offset += 1;

        buffer[offset] = ((self.seq as u8) & 0x0Fu8) | (((self.seq_count as u8) & 0x0Fu8) << 4u8);
        offset += 1;

        buffer[offset..offset + 4].copy_from_slice(&self.id.to_le_bytes());
        offset += 4;

        // Reserved
        offset += 3;

        buffer[offset..offset + 2].copy_from_slice(&self.len.to_le_bytes());
        offset += 2;

        buffer[offset..offset + 4].copy_from_slice(&self.crc.to_le_bytes());

        return buffer;
    }

    pub fn unpack(&mut self, data: &[u8]) -> Result<usize, KaonicError> {
        if data.len() < HEADER_SIZE {
            return Err(KaonicError::IncorrectSettings);
        }

        let mut offset = 0usize;

        self.packet_type = match data[offset] {
            0xBA => PacketType::Payload,
            _ => return Err(KaonicError::IncorrectSettings),
        };
        offset += 1;

        self.flags = data[1];
        offset += 1;

        self.seq = (data[2] & 0x0F) as usize;
        self.seq_count = (((data[2] & 0xF0) as u8) >> 4u8) as usize;
        offset += 1;

        self.id = u32::from_le_bytes([
            data[offset + 0],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        // Reserved
        offset += 3;

        self.len = u16::from_le_bytes([data[offset + 0], data[offset + 1]]);
        offset += 2;

        self.crc = u32::from_le_bytes([
            data[offset + 0],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        Ok(offset)
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "[tp:{:02X} id:{:0>8X} flg:{:b} len:{:0>4}B crc:{:0>8X}]",
            self.packet_type as u8, self.id, self.flags, self.len, self.crc
        )?;

        Ok(())
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Packet<const S: usize> {
    header: Header,  // Header
    frame: Frame<S>, // Payload
}

impl<const S: usize> Packet<S> {
    pub const fn new() -> Self {
        Self {
            header: Header::new(),
            frame: Frame::new(),
        }
    }

    pub fn reset(&mut self) {
        self.header = Header::new();
        self.frame.clear();
    }

    pub fn build(&mut self) {
        self.header.len = self.frame.len() as u16;
        self.header.crc = Self::calculate_crc(self.frame.as_slice());
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }

    pub fn validate(&self) -> bool {
        let actualt_crc = Self::calculate_crc(&self.frame.as_slice());
        if actualt_crc != self.header.crc {
            return false;
        }

        if self.frame.len() != self.header.len.into() {
            return false;
        }

        return true;
    }

    pub fn frame(&self) -> &Frame<S> {
        &self.frame
    }

    pub fn frame_mut(&mut self) -> &mut Frame<S> {
        &mut self.frame
    }

    fn calculate_crc(data: &[u8]) -> u32 {
        let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        crc.checksum(data)
    }
}

pub trait PacketCoder<const S: usize> {
    const MAX_PAYLOAD_SIZE: usize;

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), KaonicError>;

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), KaonicError>;
}

#[derive(Copy, Clone, Debug)]
pub struct LdpcPacketCoder<const S: usize> {
    working_buffer: [u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
    output_buffer: [u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
}

impl<const S: usize> LdpcPacketCoder<S> {
    const MAX_ENCODED_PAYLOAD_SIZE: usize = (S - (HEADER_LDPC_CODE.n() / 8));
    pub fn new() -> Self {
        Self {
            working_buffer: [0u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
            output_buffer: [0u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
        }
    }
}

impl<const S: usize> PacketCoder<S> for LdpcPacketCoder<S> {
    const MAX_PAYLOAD_SIZE: usize = (Self::MAX_ENCODED_PAYLOAD_SIZE / (PAYLOAD_LDPC_CODE.n() / 8))
        * (PAYLOAD_LDPC_CODE.k() / 8);

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), KaonicError> {
        // Reset output frame
        output.clear();

        // Encode header
        {
            let header_data = input.header.pack();
            let code = HEADER_LDPC_CODE;

            let codeword_len = code.n() / 8;
            if codeword_len > S {
                return Err(KaonicError::OutOfMemory);
            }

            let _ = code.copy_encode(&header_data[..], output.alloc_buffer(codeword_len));
        }

        // Encode payload
        {
            let code = PAYLOAD_LDPC_CODE;
            let payload_data = input.frame.as_slice();
            let mut offset = 0;

            let block_size = code.k() / 8;
            let code_block_size = code.n() / 8;

            while offset < payload_data.len() {
                let block_len = if offset + block_size < payload_data.len() {
                    block_size
                } else {
                    payload_data.len() - offset
                };

                self.output_buffer[..block_len]
                    .copy_from_slice(&payload_data[offset..offset + block_len]);

                if block_len < block_size {
                    self.output_buffer[block_len..block_len + block_size].fill(0);
                }

                let buffer = output.alloc_buffer(code_block_size);
                if buffer.len() < code_block_size {
                    return Err(KaonicError::OutOfMemory);
                }

                code.copy_encode(&self.output_buffer[..block_size], buffer);

                offset += block_len;
            }
        }

        Ok(())
    }

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), KaonicError> {
        output.reset();

        // Decode header
        {
            let code = HEADER_LDPC_CODE;
            let codeword_len = code.n() / 8;

            if input.len() < codeword_len {
                return Err(KaonicError::OutOfMemory);
            }

            let (check, _) = code.decode_bf(
                &input.as_slice()[..codeword_len],
                &mut self.output_buffer[..code.output_len()],
                &mut self.working_buffer[..code.decode_bf_working_len()],
                20,
            );

            if !check {
                return Err(KaonicError::DataCorruption);
            }

            output
                .header
                .unpack(&mut self.output_buffer[..HEADER_SIZE])?;
        }

        output.frame.clear();

        // Decode payload
        {
            // Skip header input
            let input = &input.as_slice()[HEADER_LDPC_CODE.n() / 8..];

            let code = PAYLOAD_LDPC_CODE;

            let codeword_len = code.n() / 8;

            let mut offset = 0usize;
            while offset < input.len() {
                let (check, _) = code.decode_bf(
                    &input[offset..offset + codeword_len],
                    &mut self.output_buffer[..code.output_len()],
                    &mut self.working_buffer[..code.decode_bf_working_len()],
                    20,
                );

                if !check {
                    return Err(KaonicError::DataCorruption);
                }

                output
                    .frame
                    .push_data(&self.output_buffer[..code.k() / 8])?;

                offset += codeword_len;
            }
        }

        // Resize to original payload length
        output.frame.resize(output.header.len as usize);

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    #[test]
    fn test_encode_decode_simple() {
        const SIZE: usize = 2048;

        let test_data = "@@ TEST PACKET DATA @@";
        let mut packet: Packet<SIZE> = Packet::new();
        let mut frame: Frame<SIZE> = Frame::new();

        let mut coder = LdpcPacketCoder::<SIZE>::new();

        packet
            .frame
            .push_data(test_data.as_bytes())
            .expect("packet with data");

        packet.build();

        coder.encode(&packet, &mut frame).expect("encoded frame");

        // Corrupt data
        {
            frame.as_slice_mut()[0] = 0;
            frame.as_slice_mut()[15] = 0;
            frame.as_slice_mut()[33] = 0;
            frame.as_slice_mut()[34] = 0;
            frame.as_slice_mut()[35] = 0;
            frame.as_slice_mut()[36] = 0;
            frame.as_slice_mut()[37] = 0;
            frame.as_slice_mut()[90] = 0;
            frame.as_slice_mut()[196] = 0;
            frame.as_slice_mut()[231] = 0;
        }

        coder.decode(&frame, &mut packet).expect("decoded frame");

        assert!(packet.validate());

        assert_eq!(test_data.as_bytes(), packet.frame.as_slice());
    }
}

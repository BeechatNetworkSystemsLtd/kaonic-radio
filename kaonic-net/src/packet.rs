use kaonic_radio::{error::KaonicError, frame::Frame};
use labrador_ldpc::LDPCCode;

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

pub struct Header {
    pub packet_type: PacketType,
    pub flags: u8,
    pub length: u16, // Payload length
    pub crc: u32,
}

impl Header {
    pub const fn new() -> Self {
        Self {
            packet_type: PacketType::Payload,
            flags: 0,
            length: 0,
            crc: 0,
        }
    }

    fn pack(&self) -> [u8; HEADER_SIZE] {
        let mut buffer: [u8; HEADER_SIZE] = [0; HEADER_SIZE];

        let mut offset = 0usize;
        buffer[offset] = self.packet_type as u8;
        offset += 1;
        buffer[offset] = self.flags;
        offset += 1;

        offset += 8; // Reserved

        buffer[offset..offset + 2].copy_from_slice(&self.length.to_le_bytes());
        offset += 2;

        buffer[offset..offset + 4].copy_from_slice(&self.crc.to_le_bytes());

        return buffer;
    }

    fn unpack(&mut self, data: &[u8]) -> Result<(), KaonicError> {
        if data.len() < HEADER_SIZE {
            return Err(KaonicError::IncorrectSettings);
        }

        self.packet_type = match data[0] {
            0xBA => PacketType::Payload,
            _ => return Err(KaonicError::IncorrectSettings),
        };

        self.flags = data[1];

        self.length = u16::from_le_bytes([data[HEADER_SIZE - 6], data[HEADER_SIZE - 5]]);

        self.crc = u32::from_le_bytes([
            data[HEADER_SIZE - 4],
            data[HEADER_SIZE - 3],
            data[HEADER_SIZE - 2],
            data[HEADER_SIZE - 1],
        ]);

        Ok(())
    }
}

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
        self.header.length = self.frame.len() as u16;
        self.header.crc = Self::calculate_crc(self.frame.as_slice());
    }

    pub fn validate(&self) -> bool {

        let actualt_crc = Self::calculate_crc(&self.frame.as_slice());
        if actualt_crc != self.header.crc {
            return false;
        }

        if self.frame.len() != self.header.length.into() {
            return false;
        }

        return true;
    }

    pub fn get_frame(&self) -> &Frame<S> {
        &self.frame
    }

    pub fn get_mut_frame(&mut self) -> &mut Frame<S> {
        &mut self.frame
    }

    fn calculate_crc(data: &[u8]) -> u32 {
        let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        crc.checksum(data)
    }
}

pub struct PacketCoder<const S: usize> {
    working_buffer: [u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
    output_buffer: [u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
}

impl<const S: usize> PacketCoder<S> {
    pub fn new() -> Self {
        Self {
            working_buffer: [0u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
            output_buffer: [0u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
        }
    }

    pub fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), KaonicError> {
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

            let _ = code.copy_encode(&header_data[..], output.as_buffer_mut(codeword_len));
        }

        output.push_data(input.frame.as_slice())?;

        return Ok(());

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

                let buffer = output.as_buffer_mut(code_block_size);
                if buffer.len() < code_block_size {
                    return Err(KaonicError::OutOfMemory);
                }

                code.copy_encode(&self.output_buffer[..block_size], buffer);

                offset += block_len;
            }
        }

        Ok(())
    }

    pub fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), KaonicError> {
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
        output
            .frame
            .push_data(&input.as_slice()[HEADER_LDPC_CODE.n() / 8..])?;

        output.frame.resize(output.header.length as usize);

        return Ok(());

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
        output.frame.resize(output.header.length as usize);

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

        let mut coder = PacketCoder::<SIZE>::new();

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

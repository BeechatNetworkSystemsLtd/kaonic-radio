use kaonic_frame::frame::Frame;
use labrador_ldpc::LDPCCode;

use crate::{
    error::NetworkError,
    packet::{Packet, HEADER_SIZE},
};

pub const HEADER_LDPC_CODE: LDPCCode = LDPCCode::TC256;
pub const PAYLOAD_LDPC_CODE: LDPCCode = LDPCCode::TM2048;

pub const PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE: usize = PAYLOAD_LDPC_CODE.output_len();
pub const PAYLOAD_LDPC_WORKING_BUFFER_SIZE: usize = PAYLOAD_LDPC_CODE.decode_bf_working_len();

pub trait PacketCoder<const S: usize> {
    const MAX_PAYLOAD_SIZE: usize;

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), NetworkError>;

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), NetworkError>;
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

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), NetworkError> {
        // Reset output frame
        output.clear();

        // Encode header
        {
            let header_data = input.header().pack();
            let code = HEADER_LDPC_CODE;

            let codeword_len = code.n() / 8;
            if codeword_len > S {
                return Err(NetworkError::OutOfMemory);
            }

            let _ = code.copy_encode(&header_data[..], output.alloc_buffer(codeword_len));
        }

        // Encode payload
        {
            let code = PAYLOAD_LDPC_CODE;
            let payload_data = input.frame().as_slice();
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
                    return Err(NetworkError::OutOfMemory);
                }

                code.copy_encode(&self.output_buffer[..block_size], buffer);

                offset += block_len;
            }
        }

        Ok(())
    }

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), NetworkError> {
        output.reset();

        // Decode header
        {
            let code = HEADER_LDPC_CODE;
            let codeword_len = code.n() / 8;

            if input.len() < codeword_len {
                return Err(NetworkError::OutOfMemory);
            }

            let (check, _) = code.decode_bf(
                &input.as_slice()[..codeword_len],
                &mut self.output_buffer[..code.output_len()],
                &mut self.working_buffer[..code.decode_bf_working_len()],
                20,
            );

            if !check {
                return Err(NetworkError::CorruptedData);
            }

            output
                .header_mut()
                .unpack(&mut self.output_buffer[..HEADER_SIZE])?;
        }

        output.frame_mut().clear();

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
                    return Err(NetworkError::CorruptedData);
                }

                output
                    .frame_mut()
                    .push_data(&self.output_buffer[..code.k() / 8])?;

                offset += codeword_len;
            }
        }

        // Resize to original payload length
        let len = output.header().len() as usize;
        output.frame_mut().resize(len);

        Ok(())
    }
}

#[derive(Copy, Clone, Debug)]
pub struct BinaryPacketCoder<const S: usize> {}

impl<const S: usize> BinaryPacketCoder<S> {
    pub fn new() -> Self {
        Self {}
    }
}

impl<const S: usize> PacketCoder<S> for BinaryPacketCoder<S> {
    const MAX_PAYLOAD_SIZE: usize = S - HEADER_SIZE;

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), NetworkError> {
        // Reset output frame
        output.clear();

        // Encode header
        {
            let header_data = input.header().pack();
            output.push_data(&header_data)?;
        }

        // Encode payload
        {
            let payload_data = input.frame().as_slice();
            output.push_data(&payload_data)?;
        }

        Ok(())
    }

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), NetworkError> {
        output.reset();

        let input = input.as_slice();

        // Decode header
        {
            output.header_mut().unpack(&input[..HEADER_SIZE])?;
        }

        output.frame_mut().clear();

        // Decode payload
        {
            output.frame_mut().push_data(&input[..HEADER_SIZE])?;
        }

        // Resize to original payload length
        let len = output.header().len() as usize;
        output.frame_mut().resize(len);

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
            .frame_mut()
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

        assert_eq!(test_data.as_bytes(), packet.frame().as_slice());
    }
}

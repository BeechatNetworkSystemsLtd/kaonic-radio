use kaonic_frame::frame::Frame;
use labrador_ldpc::LDPCCode;

use crate::{
    error::NetworkError,
    packet::{Packet, HEADER_SIZE},
};

pub const HEADER_LDPC_CODE: LDPCCode = LDPCCode::TC256;
/// Payload code used by firmware that predates per-frame code selection,
/// and the default for new senders so mixed networks keep working.
pub const PAYLOAD_LDPC_CODE: LDPCCode = LDPCCode::TM2048;

/// Payload forward-error-correction scheme, carried per frame in the header
/// (see [`crate::packet::Header::fec`]). Ids are wire values: `0` is what
/// older firmware writes into the (then reserved) byte, so old senders are
/// decoded as [`FecCode::Tm2048`] and old receivers understand new senders
/// as long as they stay on that code.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum FecCode {
    /// Rate 1/2, strongest; ~2x payload airtime. Wire-compatible default.
    #[default]
    Tm2048 = 0,
    /// Rate 2/3.
    Tm1536 = 1,
    /// Rate 4/5, cheapest LDPC; for strong links.
    Tm1280 = 2,
    /// Rate 1/2 short block: cheap decode, weaker correction, no puncturing.
    Tc512 = 3,
    /// Payload sent uncoded (CRC only). Header is still LDPC-protected.
    None = 4,
}

impl FecCode {
    pub const ALL: [FecCode; 5] = [
        FecCode::Tm2048,
        FecCode::Tm1536,
        FecCode::Tm1280,
        FecCode::Tc512,
        FecCode::None,
    ];

    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(FecCode::Tm2048),
            1 => Some(FecCode::Tm1536),
            2 => Some(FecCode::Tm1280),
            3 => Some(FecCode::Tc512),
            4 => Some(FecCode::None),
            _ => None,
        }
    }

    pub const fn id(self) -> u8 {
        self as u8
    }

    pub const fn ldpc(self) -> Option<LDPCCode> {
        match self {
            FecCode::Tm2048 => Some(LDPCCode::TM2048),
            FecCode::Tm1536 => Some(LDPCCode::TM1536),
            FecCode::Tm1280 => Some(LDPCCode::TM1280),
            FecCode::Tc512 => Some(LDPCCode::TC512),
            FecCode::None => None,
        }
    }

    /// Data bytes per codeword (block size the payload is cut into).
    pub const fn block_len(self) -> usize {
        match self.ldpc() {
            Some(code) => code.k() / 8,
            None => 1,
        }
    }

    /// On-air bytes per codeword.
    pub const fn codeword_len(self) -> usize {
        match self.ldpc() {
            Some(code) => code.n() / 8,
            None => 1,
        }
    }

    /// On-air payload bytes for `payload_len` data bytes.
    pub const fn encoded_len(self, payload_len: usize) -> usize {
        let block = self.block_len();
        ((payload_len + block - 1) / block) * self.codeword_len()
    }

    pub const fn name(self) -> &'static str {
        match self {
            FecCode::Tm2048 => "tm2048",
            FecCode::Tm1536 => "tm1536",
            FecCode::Tm1280 => "tm1280",
            FecCode::Tc512 => "tc512",
            FecCode::None => "none",
        }
    }
}

const fn max_usize(a: usize, b: usize) -> usize {
    if a > b {
        a
    } else {
        b
    }
}

const fn max_output_len() -> usize {
    let mut max = 0;
    let mut i = 0;
    while i < FecCode::ALL.len() {
        if let Some(code) = FecCode::ALL[i].ldpc() {
            max = max_usize(max, code.output_len());
        }
        i += 1;
    }
    max
}

const fn max_working_len() -> usize {
    let mut max = 0;
    let mut i = 0;
    while i < FecCode::ALL.len() {
        if let Some(code) = FecCode::ALL[i].ldpc() {
            max = max_usize(max, code.decode_bf_working_len());
        }
        i += 1;
    }
    max
}

pub const PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE: usize = max_output_len();
pub const PAYLOAD_LDPC_WORKING_BUFFER_SIZE: usize = max_working_len();

/// How the receiver treats an LDPC-coded payload.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum DecodePolicy {
    /// Check the packet CRC over the systematic bytes first and run the
    /// iterative decoder only when it fails. Clean frames cost a CRC.
    #[default]
    VerifyFirst,
    /// Always run the iterative decoder (reference behaviour).
    AlwaysDecode,
}

/// Receive-side counters, useful for tuning the FEC policy in the field.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CoderStats {
    /// Payloads accepted on the CRC fast path without iterative decoding.
    pub verified_fast: u64,
    /// Payloads that needed the iterative decoder (and succeeded).
    pub decoded_full: u64,
    /// Payloads the decoder could not repair.
    pub failed: u64,
    /// Headers that could not be decoded.
    pub header_failed: u64,
}

pub trait PacketCoder<const S: usize> {
    const MAX_PAYLOAD_SIZE: usize;

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), NetworkError>;

    fn decode(&mut self, input: &Frame<S>, output: &mut Packet<S>) -> Result<(), NetworkError>;
}

#[derive(Copy, Clone, Debug)]
pub struct LdpcPacketCoder<const S: usize> {
    working_buffer: [u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
    output_buffer: [u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
    tx_fec: FecCode,
    policy: DecodePolicy,
    stats: CoderStats,
}

impl<const S: usize> LdpcPacketCoder<S> {
    const MAX_ENCODED_PAYLOAD_SIZE: usize = (S - (HEADER_LDPC_CODE.n() / 8));

    pub fn new() -> Self {
        Self {
            working_buffer: [0u8; PAYLOAD_LDPC_WORKING_BUFFER_SIZE],
            output_buffer: [0u8; PAYLOAD_LDPC_OUTPUT_BUFFER_SIZE],
            tx_fec: FecCode::default(),
            policy: DecodePolicy::default(),
            stats: CoderStats::default(),
        }
    }

    pub fn with_fec(mut self, fec: FecCode) -> Self {
        self.tx_fec = fec;
        self
    }

    pub fn with_decode_policy(mut self, policy: DecodePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Code used for frames encoded from now on; received frames are always
    /// decoded with whatever their header announces.
    pub fn set_tx_fec(&mut self, fec: FecCode) {
        self.tx_fec = fec;
    }

    pub fn tx_fec(&self) -> FecCode {
        self.tx_fec
    }

    pub fn set_decode_policy(&mut self, policy: DecodePolicy) {
        self.policy = policy;
    }

    pub fn stats(&self) -> CoderStats {
        self.stats
    }

    /// Largest payload a single frame can carry with `fec`.
    pub const fn max_payload_for(fec: FecCode) -> usize {
        (Self::MAX_ENCODED_PAYLOAD_SIZE / fec.codeword_len()) * fec.block_len()
    }

    fn decode_payload(
        &mut self,
        fec: FecCode,
        input: &[u8],
        output: &mut Packet<S>,
    ) -> Result<(), NetworkError> {
        let payload_len = output.header().len() as usize;
        let expected_crc = output.header().crc();

        let Some(code) = fec.ldpc() else {
            if input.len() < payload_len {
                return Err(NetworkError::OutOfMemory);
            }
            output.frame_mut().push_data(&input[..payload_len])?;
            return Ok(());
        };

        let block_len = code.k() / 8;
        let codeword_len = code.n() / 8;
        if input.len() % codeword_len != 0 || input.len() / codeword_len * block_len < payload_len {
            return Err(NetworkError::CorruptedData);
        }

        // Fast path: the codes are systematic, so the data bytes are the
        // first `k/8` of each codeword. If they already satisfy the packet
        // CRC there is nothing for the decoder to fix.
        if self.policy == DecodePolicy::VerifyFirst {
            output.frame_mut().clear();
            let mut offset = 0usize;
            while offset < input.len() {
                output
                    .frame_mut()
                    .push_data(&input[offset..offset + block_len])?;
                offset += codeword_len;
            }
            output.frame_mut().resize(payload_len);
            if kaonic_crc::crc32(output.frame().as_slice()) == expected_crc {
                self.stats.verified_fast += 1;
                return Ok(());
            }
            output.frame_mut().clear();
        }

        let mut offset = 0usize;
        while offset < input.len() {
            let (check, _) = code.decode_bf(
                &input[offset..offset + codeword_len],
                &mut self.output_buffer[..code.output_len()],
                &mut self.working_buffer[..code.decode_bf_working_len()],
                20,
            );
            if !check {
                self.stats.failed += 1;
                return Err(NetworkError::CorruptedData);
            }
            output
                .frame_mut()
                .push_data(&self.output_buffer[..block_len])?;
            offset += codeword_len;
        }
        output.frame_mut().resize(payload_len);
        self.stats.decoded_full += 1;
        Ok(())
    }
}

impl<const S: usize> Default for LdpcPacketCoder<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const S: usize> PacketCoder<S> for LdpcPacketCoder<S> {
    // Segment size stays the legacy TM2048 capacity so every code (and every
    // receiver) can carry a segment; stronger-rate codes just use less air.
    const MAX_PAYLOAD_SIZE: usize = Self::max_payload_for(FecCode::Tm2048);

    fn encode(&mut self, input: &Packet<S>, output: &mut Frame<S>) -> Result<(), NetworkError> {
        let fec = self.tx_fec;
        if input.frame().len() > Self::max_payload_for(fec) {
            return Err(NetworkError::PayloadTooBig);
        }

        // Reset output frame
        output.clear();

        // Encode header (announces the payload code)
        {
            let mut header = *input.header();
            header.set_fec(fec.id());
            let header_data = header.pack();
            let code = HEADER_LDPC_CODE;

            let codeword_len = code.n() / 8;
            if codeword_len > S {
                return Err(NetworkError::OutOfMemory);
            }

            let _ = code.copy_encode(&header_data[..], output.alloc_buffer(codeword_len)?);
        }

        // Encode payload
        let payload_data = input.frame().as_slice();
        let Some(code) = fec.ldpc() else {
            output
                .alloc_buffer(payload_data.len())?
                .copy_from_slice(payload_data);
            return Ok(());
        };
        {
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

                let buffer = output.alloc_buffer(code_block_size)?;
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
                self.stats.header_failed += 1;
                return Err(NetworkError::CorruptedData);
            }

            output
                .header_mut()
                .unpack(&mut self.output_buffer[..HEADER_SIZE])?;
        }

        let fec = FecCode::from_id(output.header().fec()).ok_or(NetworkError::NotSupported)?;
        let payload_input = &input.as_slice()[HEADER_LDPC_CODE.n() / 8..];
        self.decode_payload(fec, payload_input, output)
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
            output.frame_mut().push_data(&input[HEADER_SIZE..])?;
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

#[cfg(test)]
mod fec_tests {
    use super::*;
    use crate::packet::Packet;

    const S: usize = 2048;

    fn packet(len: usize) -> Packet<S> {
        let mut packet = Packet::<S>::new();
        let data = alloc_vec(len);
        packet.frame_mut().push_data(&data).unwrap();
        packet.build();
        packet
    }

    fn alloc_vec(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 37 % 251) as u8).collect()
    }

    #[test]
    fn every_code_round_trips_and_verifies_fast() {
        for fec in FecCode::ALL {
            let mut coder = LdpcPacketCoder::<S>::new().with_fec(fec);
            let input = packet(806);
            let mut frame = Frame::<S>::new();
            coder.encode(&input, &mut frame).unwrap();
            assert_eq!(
                frame.len(),
                HEADER_LDPC_CODE.n() / 8 + fec.encoded_len(806),
                "{fec:?} on-air length"
            );
            let mut output = Packet::<S>::new();
            coder.decode(&frame, &mut output).unwrap();
            assert!(output.validate());
            assert_eq!(output.frame().as_slice(), input.frame().as_slice());
            assert_eq!(output.header().fec(), fec.id());
            let stats = coder.stats();
            if fec == FecCode::None {
                assert_eq!(stats.verified_fast + stats.decoded_full, 0);
            } else {
                assert_eq!(stats.verified_fast, 1, "{fec:?} should take the fast path");
            }
        }
    }

    #[test]
    fn corrupted_frame_falls_back_to_full_decode() {
        for fec in [FecCode::Tm2048, FecCode::Tm1280, FecCode::Tc512] {
            let mut coder = LdpcPacketCoder::<S>::new().with_fec(fec);
            let input = packet(500);
            let mut frame = Frame::<S>::new();
            coder.encode(&input, &mut frame).unwrap();
            // Flip a few payload bits (inside the systematic part).
            let payload_start = HEADER_LDPC_CODE.n() / 8;
            let bytes = frame.as_slice_mut();
            bytes[payload_start + 3] ^= 0x01;
            bytes[payload_start + 40] ^= 0x10;
            let mut output = Packet::<S>::new();
            coder.decode(&frame, &mut output).unwrap();
            assert!(output.validate(), "{fec:?} repaired");
            assert_eq!(output.frame().as_slice(), input.frame().as_slice());
            assert_eq!(coder.stats().decoded_full, 1);
            assert_eq!(coder.stats().verified_fast, 0);
        }
    }

    #[test]
    fn legacy_header_decodes_as_tm2048() {
        // A sender that predates the fec byte writes 0 there.
        let mut coder = LdpcPacketCoder::<S>::new();
        let input = packet(100);
        let mut frame = Frame::<S>::new();
        coder.encode(&input, &mut frame).unwrap();
        let mut output = Packet::<S>::new();
        coder.decode(&frame, &mut output).unwrap();
        assert_eq!(output.header().fec(), 0);
        assert_eq!(FecCode::from_id(0), Some(FecCode::Tm2048));
        assert_eq!(FecCode::from_id(9), None);
    }

    #[test]
    fn segment_size_is_legacy_capacity() {
        assert_eq!(
            <LdpcPacketCoder<S> as PacketCoder<S>>::MAX_PAYLOAD_SIZE,
            896
        );
        assert!(LdpcPacketCoder::<S>::max_payload_for(FecCode::Tm1280) > 896);
    }
}

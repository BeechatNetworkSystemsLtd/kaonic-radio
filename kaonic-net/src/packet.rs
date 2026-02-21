use core::fmt;

use kaonic_frame::frame::{Frame, FrameSegment};

use crate::error::NetworkError;

pub const HEADER_SIZE: usize = 16;

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
    Encoded = 0b0000_0001,
    /// Large payload is split into segments
    Segmented = 0b0000_0010,
    ///
    Acknowledge = 0b0000_0100,
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

    pub fn pack(&self) -> [u8; HEADER_SIZE] {
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

    pub fn unpack(&mut self, data: &[u8]) -> Result<usize, NetworkError> {
        if data.len() < HEADER_SIZE {
            return Err(NetworkError::OutOfMemory);
        }

        let mut offset = 0usize;

        self.packet_type = match data[offset] {
            0xBA => PacketType::Payload,
            _ => return Err(NetworkError::NotSupported),
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

pub struct AssembledPacket<'a, const S: usize, const R: usize> {
    id: PacketId,
    frame: &'a FrameSegment<S, R>,
}

impl<'a, const S: usize, const R: usize> AssembledPacket<'a, S, R> {
    pub fn new(id: PacketId, frame: &'a FrameSegment<S, R>) -> Self {
        Self { id, frame }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.frame.as_slice()
    }

    pub fn frame(&self) -> &'a FrameSegment<S, R> {
        self.frame
    }

    pub fn id(&self) -> PacketId {
        self.id
    }
}

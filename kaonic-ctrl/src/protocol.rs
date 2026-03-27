use std::sync::Arc;

use kaonic_frame::frame::FrameSegment;
use radio_common::{Modulation, RadioConfig};
use rand::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::{
    error::ControllerError,
    peer::{PeerCoder, PeerMessage, PeerMessageId},
};

pub const CTRL_PATTERN: u16 = 0xBACE;
pub const RADIO_FRAME_SIZE: usize = 2048;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RadioFrame {
    #[serde(with = "serde_bytes")]
    pub data: [u8; RADIO_FRAME_SIZE],
    pub len: u16,
}

impl RadioFrame {
    pub fn new() -> Self {
        Self {
            data: [0u8; RADIO_FRAME_SIZE],
            len: 0,
        }
    }

    pub fn new_from_frame<const S: usize, const R: usize>(frame: &FrameSegment<S, R>) -> Self {
        let mut radio_frame = Self::new();
        let len = core::cmp::min(frame.len(), radio_frame.data.len());

        radio_frame.data[..len].copy_from_slice(&frame.as_slice()[..len]);
        radio_frame.len = len as u16;

        radio_frame
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..(self.len as usize)]
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TransmitModule {
    pub module: usize,
    pub frame: RadioFrame,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ReceiveModule {
    pub module: usize,
    pub frame: RadioFrame,
    pub rssi: i8,
}

impl ReceiveModule {
    pub fn new() -> Self {
        Self {
            module: 0,
            frame: RadioFrame::new(),
            rssi: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetInfoResponse {
    pub module_count: usize,
    pub serial: String,
    pub mtu: usize,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetStatisticsRequest {
    pub module: usize,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct GetStatisticsResponse {
    pub module: usize,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetModulationRequest {
    pub module: usize,
    pub modulation: Modulation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetModulationRequest {
    pub module: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetModulationResponse {
    pub module: usize,
    pub modulation: Modulation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetRadioConfigRequest {
    pub module: usize,
    pub config: RadioConfig,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetRadioConfigRequest {
    pub module: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetRadioConfigResponse {
    pub module: usize,
    pub config: RadioConfig,
}

//***********************************************************************************************//

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Payload {
    Ping,
    Pong,
    TransmitModuleRequest(TransmitModule),
    TransmitModuleResponse,
    ReceiveModule(ReceiveModule),
    ScanRequest,
    SetRadioConfigRequest(SetRadioConfigRequest),
    SetRadioConfigResponse,
    GetRadioConfigRequest(GetRadioConfigRequest),
    GetRadioConfigResponse(GetRadioConfigResponse),
    SetModulationRequest(SetModulationRequest),
    SetModulationResponse,
    GetModulationRequest(GetModulationRequest),
    GetModulationResponse(GetModulationResponse),
    GetInfoRequest,
    GetInfoResponse(GetInfoResponse),
    GetStatisticsRequest(GetStatisticsRequest),
    GetStatisticsResponse(GetStatisticsResponse),
    NotImplemented,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    // should be equal to CTRL_PATTERN
    pub pattern: u16,
    pub version: u16,
    pub id: u32,
    pub flags: u32,
    pub payload: Payload,
}

impl Message {
    pub fn new() -> Self {
        Self {
            pattern: CTRL_PATTERN,
            version: 0,
            id: 0,
            flags: 0,
            payload: Payload::ScanRequest,
        }
    }
}

impl PeerMessage for Message {
    fn message_id(&self) -> PeerMessageId {
        PeerMessageId(self.id)
    }
}

impl PeerMessage for std::sync::Arc<Message> {
    fn message_id(&self) -> PeerMessageId {
        PeerMessageId(self.id)
    }
}

impl PeerMessage for Box<Message> {
    fn message_id(&self) -> PeerMessageId {
        PeerMessageId(self.id)
    }
}

#[derive(Debug)]
pub struct MessageBuilder {
    message: Message,
}

impl MessageBuilder {
    pub fn new() -> Self {
        Self {
            message: Message {
                pattern: CTRL_PATTERN,
                version: 0,
                flags: 0,
                id: 0,
                payload: Payload::ScanRequest,
            },
        }
    }

    pub fn with_payload(mut self, payload: Payload) -> Self {
        self.message.payload = payload;
        self
    }

    pub fn with_id(mut self, id: u32) -> Self {
        self.message.id = id;
        self
    }

    pub fn with_rnd_id<RNG: CryptoRng + RngCore + Copy>(mut self, mut rng: RNG) -> Self {
        self.message.id = rng.next_u32();
        self
    }

    pub fn build(self) -> Message {
        self.message
    }
}

#[derive(Debug)]
pub struct MessageCoder<const MTU: usize, const R: usize> {
    buffer: Vec<u8>,
}

impl<const MTU: usize, const R: usize> MessageCoder<MTU, R> {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(MTU * R),
        }
    }
}

impl<const MTU: usize, const R: usize> PeerCoder<Message, MTU, R> for MessageCoder<MTU, R> {
    fn serialize(
        &mut self,
        message: &Message,
        frame: &mut FrameSegment<MTU, R>,
    ) -> Result<(), ControllerError> {
        frame.clear();

        frame.push_data(&message.pattern.to_le_bytes()[..])?;
        frame.push_data(&message.version.to_le_bytes()[..])?;
        frame.push_data(&message.id.to_le_bytes()[..])?;
        frame.push_data(&message.flags.to_le_bytes()[..])?;

        self.buffer.clear();

        let mut serializer = rmp_serde::Serializer::new(&mut self.buffer);

        message
            .payload
            .serialize(&mut serializer)
            .map_err(|_| ControllerError::OutOfMemory)?;

        frame
            .alloc_buffer(self.buffer.len())?
            .copy_from_slice(self.buffer.as_slice());

        Ok(())
    }

    fn deserialize<'a>(
        &mut self,
        packet: &kaonic_net::packet::AssembledPacket<'a, MTU, R>,
    ) -> Result<Message, ControllerError> {
        let mut message = Message::new();

        let input_data = packet.as_slice();

        let mut offset = 0usize;

        if input_data.len() < offset {
            return Err(ControllerError::DecodeError);
        }

        message.pattern = u16::from_le_bytes([input_data[offset + 0], input_data[offset + 1]]);

        // Check if message has pattern
        if message.pattern != CTRL_PATTERN {
            return Err(ControllerError::DecodeError);
        }

        offset += 2;

        if input_data.len() < offset {
            return Err(ControllerError::DecodeError);
        }

        message.version = u16::from_le_bytes([input_data[offset + 0], input_data[offset + 1]]);

        offset += 2;

        if input_data.len() < offset {
            return Err(ControllerError::DecodeError);
        }

        message.id = u32::from_le_bytes([
            input_data[offset + 0],
            input_data[offset + 1],
            input_data[offset + 2],
            input_data[offset + 3],
        ]);

        offset += 4;

        if input_data.len() < offset {
            return Err(ControllerError::DecodeError);
        }

        message.flags = u32::from_le_bytes([
            input_data[offset + 0],
            input_data[offset + 1],
            input_data[offset + 2],
            input_data[offset + 3],
        ]);

        offset += 4;

        if input_data.len() < offset {
            return Err(ControllerError::DecodeError);
        }

        let payload_data = &input_data[offset..];

        message.payload =
            rmp_serde::from_slice(payload_data).map_err(|_| ControllerError::DecodeError)?;

        Ok(message)
    }
}

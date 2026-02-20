use kaonic_frame::frame::{Frame, FrameSegment};
use radio_common::Modulation;
use rand::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::{
    error::ControllerError,
    peer::{PeerCoder, PeerMessage},
};

pub const CTRL_PATTERN: u16 = 0xBACE;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(packed)]
pub struct TransmitModule {
    pub module: u16,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(packed)]
pub struct GetInfoRequest {}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(packed)]
pub struct GetInfoResponse {}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(packed)]
pub struct SetModulationRequest {
    pub module: u16,
    pub modulation: Modulation,
}

//***********************************************************************************************//

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Payload {
    TransmitModule,
    TransmitNetwork,
    ReceiveModule,
    ReceiveNetwork,
    ScanRequest,
    SetModulationRequest(SetModulationRequest),
    GetInfoRequest(GetInfoRequest),
    GetInfoResponse(GetInfoResponse),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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
    fn message_id(&self) -> u32 {
        self.id
    }
}

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

pub fn encode_message<'a, const S: usize, const R: usize>(
    message: &Message,
    frame: &'a mut FrameSegment<S, R>,
) -> Result<&'a FrameSegment<S, R>, ControllerError> {
    frame.clear();

    frame.push_data(&message.pattern.to_le_bytes()[..])?;
    frame.push_data(&message.version.to_le_bytes()[..])?;
    frame.push_data(&message.id.to_le_bytes()[..])?;
    frame.push_data(&message.flags.to_le_bytes()[..])?;

    let meta_len = frame.len();
    let mut buffer = frame.alloc_max_buffer();

    let mut serializer = rmp_serde::Serializer::new(&mut buffer);

    message
        .payload
        .serialize(&mut serializer)
        .map_err(|_| ControllerError::OutOfMemory)?;

    let payload_len = serializer.into_inner().len();

    frame.resize(meta_len + payload_len);

    Ok(frame)
}

pub fn decode_message<'a, const S: usize, const R: usize>(
    frame: &FrameSegment<S, R>,
    message: &'a mut Message,
) -> Result<&'a Message, ControllerError> {
    let input_data = frame.as_slice();
    let mut offset = 0usize;

    if input_data.len() < offset {
        return Err(ControllerError::DecodeError);
    }

    message.pattern = u16::from_le_bytes([input_data[offset + 0], input_data[offset + 1]]);

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

    Ok(message)
}

pub struct MessageCoder<const MTU: usize, const R: usize> {}

impl<const MTU: usize, const R: usize> MessageCoder<MTU, R> {
    pub fn new() -> Self {
        Self {}
    }
}

impl<const MTU: usize, const R: usize> PeerCoder<Message, MTU, R> for MessageCoder<MTU, R> {
    fn serialize(
        &self,
        item: &Message,
        frame: &mut FrameSegment<MTU, R>,
    ) -> Result<(), ControllerError> {
        encode_message(item, frame)?;
        Ok(())
    }

    fn deserialize<'a>(
        &self,
        packet: &kaonic_net::packet::AssembledPacket<'a, MTU, R>,
    ) -> Result<Message, ControllerError> {
        let mut message = Message::new();

        decode_message(packet.frame(), &mut message)?;

        Ok(message)
    }
}

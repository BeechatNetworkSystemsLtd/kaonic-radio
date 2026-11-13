use kaonic_frame::frame::FrameSegment;
use radio_common::{Accelerator, Antenna, Modulation, RadioBand, RadioConfig};
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
        let len = core::cmp::min(usize::from(self.len), self.data.len());
        &self.data[..len]
    }

    pub fn validate(&self) -> Result<(), ControllerError> {
        if usize::from(self.len) > self.data.len() {
            return Err(ControllerError::DecodeError);
        }

        Ok(())
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetAccelerationRequest {
    pub module: usize,
    pub acceleration: Accelerator,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetAccelerationRequest {
    pub module: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetAccelerationResponse {
    pub module: usize,
    pub acceleration: Accelerator,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetAntennaRequest {
    pub module: usize,
    pub band: RadioBand,
    pub antenna: Antenna,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetAntennaRequest {
    pub module: usize,
    pub band: RadioBand,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetAntennaResponse {
    pub module: usize,
    pub band: RadioBand,
    pub antenna: Antenna,
}

/// One sub-GHz filter of a radio's front end. A filter is known by its index
/// in `GetFiltersResponse::filters`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterInfo {
    pub label: String,
    /// Passband, in kHz.
    pub min_khz: u32,
    pub max_khz: u32,
}

/// A frequency range a radio's front end is built for, per band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BandRangeInfo {
    pub band: RadioBand,
    pub min_khz: u32,
    pub max_khz: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GetFiltersRequest {
    pub module: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetFiltersResponse {
    pub module: usize,
    /// Empty when the radio has no filter control.
    pub filters: Vec<FilterInfo>,
    /// An index into `filters`; `None` is auto.
    pub selected: Option<u8>,
    /// The filter in use now; `None` without filters.
    pub active: Option<u8>,
    /// What the front end is built for, for display; empty when not known.
    pub ranges: Vec<BandRangeInfo>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetFilterRequest {
    pub module: usize,
    /// An index into the module's filters; `None` is auto. Also re-applies
    /// the filter for the current frequency.
    pub filter: Option<u8>,
}

//***********************************************************************************************//

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Payload {
    Ping,
    Pong,
    TransmitModuleRequest(TransmitModule),
    TransmitModuleResponse,
    TransmitModuleEvent(TransmitModule),
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
    // New variants appended at the end to keep rmp-serde's index-based variant encoding stable.
    SetAccelerationRequest(SetAccelerationRequest),
    SetAccelerationResponse,
    GetAccelerationRequest(GetAccelerationRequest),
    GetAccelerationResponse(GetAccelerationResponse),
    SetAntennaRequest(SetAntennaRequest),
    SetAntennaResponse,
    GetAntennaRequest(GetAntennaRequest),
    GetAntennaResponse(GetAntennaResponse),
    GetFiltersRequest(GetFiltersRequest),
    GetFiltersResponse(GetFiltersResponse),
    SetFilterRequest(SetFilterRequest),
    SetFilterResponse,
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

    fn validate(&self) -> Result<(), ControllerError> {
        self.payload.validate()
    }
}

impl Payload {
    fn validate(&self) -> Result<(), ControllerError> {
        match self {
            Payload::TransmitModuleRequest(tx) => tx.frame.validate(),
            Payload::TransmitModuleEvent(tx) => tx.frame.validate(),
            Payload::ReceiveModule(rx) => rx.frame.validate(),
            _ => Ok(()),
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
        let header_size = core::mem::size_of_val(&message.pattern)
            + core::mem::size_of_val(&message.version)
            + core::mem::size_of_val(&message.id)
            + core::mem::size_of_val(&message.flags);

        if input_data.len() < header_size {
            return Err(ControllerError::DecodeError);
        }

        message.pattern = u16::from_le_bytes([input_data[offset + 0], input_data[offset + 1]]);

        // Check if message has pattern
        if message.pattern != CTRL_PATTERN {
            return Err(ControllerError::DecodeError);
        }

        offset += 2;

        message.version = u16::from_le_bytes([input_data[offset + 0], input_data[offset + 1]]);

        offset += 2;

        message.id = u32::from_le_bytes([
            input_data[offset + 0],
            input_data[offset + 1],
            input_data[offset + 2],
            input_data[offset + 3],
        ]);

        offset += 4;

        message.flags = u32::from_le_bytes([
            input_data[offset + 0],
            input_data[offset + 1],
            input_data[offset + 2],
            input_data[offset + 3],
        ]);

        offset += 4;

        let payload_data = &input_data[offset..];

        message.payload =
            rmp_serde::from_slice(payload_data).map_err(|_| ControllerError::DecodeError)?;
        message.validate()?;

        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(payload: &Payload) -> Vec<u8> {
        let mut out = Vec::new();
        payload
            .serialize(&mut rmp_serde::Serializer::new(&mut out))
            .unwrap();
        out
    }

    /// rmp-serde names a variant by its name and a struct by its fields in
    /// order; clients that do not link this crate (the hub) encode the same.
    #[test]
    fn filter_messages_have_a_fixed_encoding() {
        let get = Payload::GetFiltersRequest(GetFiltersRequest { module: 1 });
        let mut expected = vec![0x81, 0xb1];
        expected.extend_from_slice(b"GetFiltersRequest");
        expected.extend_from_slice(&[0x91, 0x01]);
        assert_eq!(encode(&get), expected);

        let set = Payload::SetFilterRequest(SetFilterRequest { module: 0, filter: None });
        let mut expected = vec![0x81, 0xb0];
        expected.extend_from_slice(b"SetFilterRequest");
        expected.extend_from_slice(&[0x92, 0x00, 0xc0]);
        assert_eq!(encode(&set), expected);

        let set = Payload::SetFilterRequest(SetFilterRequest { module: 1, filter: Some(2) });
        let mut expected = vec![0x81, 0xb0];
        expected.extend_from_slice(b"SetFilterRequest");
        expected.extend_from_slice(&[0x92, 0x01, 0x02]);
        assert_eq!(encode(&set), expected);

        let mut expected = vec![0xb1];
        expected.extend_from_slice(b"SetFilterResponse");
        assert_eq!(encode(&Payload::SetFilterResponse), expected);
    }

    #[test]
    fn filter_response_round_trips() {
        let response = GetFiltersResponse {
            module: 1,
            filters: vec![FilterInfo {
                label: "Wide".into(),
                min_khz: 730_000,
                max_khz: 960_000,
            }],
            selected: None,
            active: Some(0),
            ranges: vec![BandRangeInfo {
                band: RadioBand::Band09,
                min_khz: 863_000,
                max_khz: 928_000,
            }],
        };
        let bytes = encode(&Payload::GetFiltersResponse(response.clone()));

        match rmp_serde::from_slice(&bytes).unwrap() {
            Payload::GetFiltersResponse(decoded) => assert_eq!(decoded, response),
            other => panic!("{other:?}"),
        }

        // What a client without this crate reads (the hub's commd client).
        let mut expected = vec![0x81, 0xb2];
        expected.extend_from_slice(b"GetFiltersResponse");
        expected.extend_from_slice(&[0x95, 0x01, 0x91, 0x93, 0xa4]);
        expected.extend_from_slice(b"Wide");
        expected.extend_from_slice(&[0xce, 0x00, 0x0b, 0x23, 0x90]);
        expected.extend_from_slice(&[0xce, 0x00, 0x0e, 0xa6, 0x00]);
        expected.extend_from_slice(&[0xc0, 0x00]);
        expected.extend_from_slice(&[0x91, 0x93, 0xa6]);
        expected.extend_from_slice(b"Band09");
        expected.extend_from_slice(&[0xce, 0x00, 0x0d, 0x2b, 0x18]);
        expected.extend_from_slice(&[0xce, 0x00, 0x0e, 0x29, 0x00]);
        assert_eq!(bytes, expected);
    }
}

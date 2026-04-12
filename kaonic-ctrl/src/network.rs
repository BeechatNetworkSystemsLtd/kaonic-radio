use std::time::Instant;

use kaonic_frame::frame::{Frame, FrameSegment};
use kaonic_net::{
    NetworkTime, coder::BinaryPacketCoder, network::Network, packet::AssembledPacket,
};
use rand::{CryptoRng, RngCore};

use crate::error::ControllerError;

const CONTROLLER_NETWORK_QUEUE_SIZE: usize = 16;

pub type ControllerCoder<const MTU: usize> = BinaryPacketCoder<MTU>;

#[derive(Debug)]
pub struct ControllerNetwork<const MTU: usize, const R: usize> {
    network: Network<MTU, R, CONTROLLER_NETWORK_QUEUE_SIZE, ControllerCoder<MTU>>,
}

impl<const MTU: usize, const R: usize> ControllerNetwork<MTU, R> {
    pub fn new() -> Self {
        Self {
            network: Network::new(ControllerCoder::new()),
        }
    }

    pub fn receive<'a>(
        &mut self,
        rx_frame: &Frame<MTU>,
        output_frame: &'a mut FrameSegment<MTU, R>,
    ) -> Result<AssembledPacket<'a, MTU, R>, ControllerError> {
        let current_time = Self::current_time();

        self.network.receive(current_time, &rx_frame)?;

        let packet = self.network.process(current_time, output_frame)?;

        Ok(packet)
    }

    pub fn transmit<'a, RNG: CryptoRng + RngCore + Copy>(
        &mut self,
        data: &[u8],
        rng: RNG,
        output_frames: &'a mut [Frame<MTU>],
    ) -> Result<&'a [Frame<MTU>], ControllerError> {
        let frames = self.network.transmit(data, rng, output_frames)?;

        Ok(frames)
    }

    fn current_time() -> NetworkTime {
        crate::system_time()
    }
}

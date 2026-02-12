use rand::{CryptoRng, RngCore};

use crate::{error::NetworkError, packet::PacketId};

pub struct Generator {}

impl Generator {
    pub fn generate_packet_id<R: CryptoRng + RngCore + Copy>(
        rng: R,
    ) -> Result<PacketId, NetworkError> {
        let mut bytes = {
            let packet_id: PacketId = 0;
            packet_id.to_ne_bytes()
        };

        Self::generate_payload(rng, &mut bytes[..])?;

        Ok(PacketId::from_ne_bytes(bytes))
    }

    pub fn generate_payload<R: CryptoRng + RngCore + Copy>(
        mut rng: R,
        output: &mut [u8],
    ) -> Result<(), NetworkError> {
        rng.fill_bytes(output);

        Ok(())
    }
}

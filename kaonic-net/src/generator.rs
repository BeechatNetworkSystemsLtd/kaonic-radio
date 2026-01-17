use kaonic_radio::error::KaonicError;
use rand::TryCryptoRng;

use crate::packet::PacketId;

pub struct Generator {}

impl Generator {
    pub fn generate_packet_id<R: TryCryptoRng + Copy>(rng: R) -> Result<PacketId, KaonicError> {
        let mut bytes = {
            let packet_id: PacketId = 0;
            packet_id.to_ne_bytes()
        };

        Self::generate_payload(rng, &mut bytes[..])?;

        Ok(PacketId::from_ne_bytes(bytes))
    }

    pub fn generate_payload<R: TryCryptoRng + Copy>(
        mut rng: R,
        output: &mut [u8],
    ) -> Result<(), KaonicError> {
        rng.try_fill_bytes(output)
            .map_err(|_| KaonicError::DataCorruption)?;

        Ok(())
    }
}

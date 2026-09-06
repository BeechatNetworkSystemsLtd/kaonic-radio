pub mod coder;
pub mod demuxer;
pub mod error;
pub mod generator;
pub mod muxer;
pub mod network;
pub mod packet;
pub mod request;

pub type NetworkTime = u128;

pub fn network_time_elapsed(
    start_time: NetworkTime,
    current_time: NetworkTime,
    duration: core::time::Duration,
) -> bool {
    let interval_time = start_time + duration.as_millis();
    current_time > interval_time
}

#[cfg(test)]
mod tests {

    use kaonic_frame::frame::{Frame, FrameSegment};
    use rand::rngs::OsRng;

    use crate::{
        coder::{LdpcPacketCoder, PacketCoder},
        demuxer::Demuxer,
        generator::Generator,
        muxer::Muxer,
        network::Network,
        packet::Packet,
    };

    const FRAME_SIZE: usize = 2048;
    const MAX_SEGMENTS_COUNT: usize = 3;

    #[test]
    fn test_multiplex_basic() {
        let rng = OsRng;

        let original_data = {
            let mut data = [0u8; 2048];
            Generator::generate_payload(rng, &mut data[..]).expect("generated payload");
            data
        };

        let original_packet_id = Generator::generate_packet_id(rng).expect("generated packet id");

        type Coder = LdpcPacketCoder<FRAME_SIZE>;
        let mut coder = Coder::new();

        let mut demuxer = Demuxer::<FRAME_SIZE, MAX_SEGMENTS_COUNT>::new(Coder::MAX_PAYLOAD_SIZE);

        let mut muxer = Muxer::<FRAME_SIZE, MAX_SEGMENTS_COUNT, 6>::new();

        let mut packets = [Packet::new(); MAX_SEGMENTS_COUNT];

        let demux_packets = demuxer
            .demultiplex(original_packet_id, &original_data[..], &mut packets[..])
            .expect("segmented data");

        let mut transfer_packet = Packet::new();
        let mut transfer_frame = Frame::new();
        let mut received_frame = FrameSegment::<FRAME_SIZE, MAX_SEGMENTS_COUNT>::new();
        for packet in demux_packets {
            assert!(packet.validate());

            coder
                .encode(packet, &mut transfer_frame)
                .expect("encoded frame");

            coder
                .decode(&transfer_frame, &mut transfer_packet)
                .expect("decoded packet");

            assert!(transfer_packet.validate());

            muxer
                .multiplex(1, &transfer_packet)
                .expect("consumed packet");
        }

        let received_data = muxer
            .process(&mut received_frame)
            .expect("received full frame")
            .as_slice()
            .to_vec();

        assert_eq!(received_data.len(), original_data.len());
        assert_eq!(received_data, original_data);

        assert!(muxer.process(&mut received_frame).is_err());
    }

    #[test]
    fn test_network() {
        let rng = OsRng;

        let original_data = {
            let mut data = [0u8; 2048];
            Generator::generate_payload(rng, &mut data[..]).expect("generated payload");
            data
        };

        type Coder = LdpcPacketCoder<FRAME_SIZE>;
        let mut tx = Network::<FRAME_SIZE, MAX_SEGMENTS_COUNT, 6, Coder>::new(Coder::new());
        let mut rx = Network::<FRAME_SIZE, MAX_SEGMENTS_COUNT, 6, Coder>::new(Coder::new());

        let mut frames = [Frame::new(); MAX_SEGMENTS_COUNT];
        let frames = tx
            .transmit(&original_data[..], rng, &mut frames)
            .expect("demuxed frames");

        let mut segment = FrameSegment::<FRAME_SIZE, MAX_SEGMENTS_COUNT>::new();
        let mut received = None;
        for (i, frame) in frames.iter().enumerate() {
            rx.receive(i as u128, frame).expect("frame accepted");
            if let Ok(packet) = rx.process(i as u128, &mut segment) {
                received = Some(packet.as_slice().to_vec());
            }
        }
        assert_eq!(received.as_deref(), Some(&original_data[..]));
    }
}

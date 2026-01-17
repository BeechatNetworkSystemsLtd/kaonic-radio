pub mod demuxer;
pub mod generator;
pub mod muxer;
pub mod packet;

#[cfg(test)]
mod tests {

    use kaonic_radio::frame::{Frame, FrameSegment};
    use rand::rngs::OsRng;

    use crate::{
        demuxer::Demuxer,
        generator::Generator,
        muxer::Muxer,
        packet::{LdpcPacketCoder, Packet, PacketCoder},
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

        let mut demuxer =
            Demuxer::<FRAME_SIZE, MAX_SEGMENTS_COUNT, { Coder::MAX_PAYLOAD_SIZE }>::new();

        println!(
            "Demuxer:\n\r\tmax_payload_len:{}\n\r\tmax_packet_payload_size:{}\n\r",
            demuxer.max_payload_size(),
            demuxer.max_packet_payload_size()
        );

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
            .process(1, &mut received_frame)
            .expect("received full frame")
            .as_slice();

        assert_eq!(received_data.len(), original_data.len());
        assert_eq!(received_data, original_data);

        assert!(muxer.process(1, &mut received_frame).is_err());
    }
}

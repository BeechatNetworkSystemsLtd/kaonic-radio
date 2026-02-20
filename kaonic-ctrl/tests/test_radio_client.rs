use kaonic_ctrl::{
    client::Client,
    peer::{Peer, NETWORK_MTU},
    protocol::MessageCoder,
};
use radio_common::Modulation;
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

const SEGMENTS_COUNT: usize = 1;

#[tokio::test]
async fn test_radio_client_bind() {
    let cancel = CancellationToken::new();

    let socket = UdpSocket::bind("0.0.0.0:9080").await.expect("socket bound");

    let peer = Peer::new(
        socket,
        "192.168.10.1:9090",
        MessageCoder::<NETWORK_MTU, SEGMENTS_COUNT>::new(),
    );

    let client = Client::<_, NETWORK_MTU, SEGMENTS_COUNT>::new(
        peer.tx_send(),
        peer.rx_recv(),
        cancel.clone(),
    )
    .await
    .expect("client");

    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _ = peer.serve(cancel).await;
        });
    }

    // let mut client = RadioClient::<1024, 6>::bind("192.168.10.1:9090", 9080, cancel.clone())
    //     .await
    //     .expect("radio was bound");
    //
    // client
    //     .set_modulation(&Modulation::Ofdm(
    //         radio_common::modulation::OfdmModulation::default(),
    //     ))
    //     .await;
    //
    cancel.cancel();
}

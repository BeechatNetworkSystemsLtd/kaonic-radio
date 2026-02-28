use kaonic_ctrl::{
    client::Client, peer::NETWORK_MTU, protocol::MessageCoder, radio::RadioClient, server::Server,
};
use kaonic_frame::frame::Frame;
use tokio_util::sync::CancellationToken;

const SEGMENTS_COUNT: usize = 4;

// #[tokio::test]
// async fn test_server_client_echo() {
//     let _ = env_logger::builder()
//         .is_test(true)
//         .filter_level(log::LevelFilter::Trace)
//         .try_init();

//     let cancel = CancellationToken::new();

//     let mut server = Server::listen(
//         "0.0.0.0:9090".parse().unwrap(),
//         MessageCoder::<NETWORK_MTU, SEGMENTS_COUNT>::new(),
//         cancel.clone(),
//     )
//     .await
//     .expect("server");

//     let mut client = Client::connect(
//         "0.0.0.0:9091".parse().unwrap(),
//         "127.0.0.1:9090".parse().unwrap(),
//         MessageCoder::<NETWORK_MTU, SEGMENTS_COUNT>::new(),
//         cancel.clone(),
//     )
//     .await
//     .expect("client");

//     // Start server routine
//     {
//         let cancel = cancel.clone();
//         tokio::spawn(async move {
//             loop {
//                 tokio::select! {
//                     Ok(req) = server.request() => {
//                         log::info!("new request");
//                         let res = *req.message();
//                         let _ = req.response(res);
//                     }
//                     _ = cancel.cancelled() => {
//                             break;
//                     }
//                 }
//             }
//         });
//     }

//     client
//         .request(
//             MessageBuilder::new().with_rnd_id(OsRng).build(),
//             core::time::Duration::from_secs(10),
//         )
//         .await
//         .expect("response");

//     cancel.cancel();
// }

#[tokio::test]
async fn test_radio_client() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Trace)
        .try_init();

    let cancel = CancellationToken::new();

    let mut server = Server::listen(
        "0.0.0.0:9090".parse().unwrap(),
        MessageCoder::<NETWORK_MTU, SEGMENTS_COUNT>::new(),
        cancel.clone(),
    )
    .await
    .expect("server");

    let client = Client::connect(
        "0.0.0.0:9091".parse().unwrap(),
        "127.0.0.1:9090".parse().unwrap(),
        MessageCoder::<NETWORK_MTU, SEGMENTS_COUNT>::new(),
        cancel.clone(),
    )
    .await
    .expect("client");

    let mut radio_client = RadioClient::new(client, cancel.clone())
        .await
        .expect("radio client");

    // Start server routine
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Ok(req) = server.request() => {
                        log::info!("new request");
                        let res = *req.message();
                        let _ = req.response(res);
                    }
                    _ = cancel.cancelled() => {
                            break;
                    }
                }
            }
        });
    }

    radio_client
        .transmit(0, &Frame::new())
        .await
        .expect("transmit");

    cancel.cancel();
}

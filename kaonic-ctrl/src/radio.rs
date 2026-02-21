use kaonic_frame::frame::Frame;
use radio_common::Modulation;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    error::ControllerError,
    protocol::{Message, MessageBuilder, Payload, RadioFrame, ReceiveModule, RADIO_FRAME_SIZE},
};

pub const DEFAULT_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(6);

pub struct RadioClient {
    module_rx_send: broadcast::Sender<ReceiveModule>,
    cancel: CancellationToken,
    client: Client<Message>,
}

impl RadioClient {
    pub async fn new(
        client: Client<Message>,
        cancel: CancellationToken,
    ) -> Result<Self, ControllerError> {
        let rx_recv = client.receive();

        let (module_rx_send, _) = broadcast::channel(8);

        tokio::spawn(Self::listen_rx(
            rx_recv,
            module_rx_send.clone(),
            cancel.clone(),
        ));

        Ok(Self {
            module_rx_send,
            client,
            cancel,
        })
    }

    pub fn module_receive(&self) -> broadcast::Receiver<ReceiveModule> {
        self.module_rx_send.subscribe()
    }

    pub async fn transmit(
        &mut self,
        module: u16,
        frame: &Frame<RADIO_FRAME_SIZE>,
    ) -> Result<(), ControllerError> {
        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::TransmitModule(crate::protocol::TransmitModule {
                        module,
                        frame: RadioFrame::new_from_frame(frame),
                    }))
                    .build(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        Ok(())
    }

    pub async fn set_modulation(&mut self, modulation: Modulation) -> Result<(), ControllerError> {
        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::SetModulationRequest(
                        crate::protocol::SetModulationRequest {
                            module: 0,
                            modulation,
                        },
                    ))
                    .build(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        Ok(())
    }

    async fn listen_rx(
        mut rx_recv: broadcast::Receiver<Message>,
        module_rx_send: broadcast::Sender<ReceiveModule>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                Ok(message) = rx_recv.recv() => {
                    match message.payload {
                        Payload::ReceiveModule(rx) => {
                            let _ = module_rx_send.send(rx);
                        },
                        _ => {}
                    }
                }
                _ = cancel.cancelled() => {

                }
            }
        }
    }

    pub fn cancel(&mut self) {
        self.client.cancel();
        self.cancel.cancel();
    }
}

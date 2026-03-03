use std::sync::Arc;

use kaonic_frame::frame::Frame;
use radio_common::{Modulation, RadioConfig};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{
    client::Client,
    error::ControllerError,
    protocol::{Message, MessageBuilder, Payload, RadioFrame, ReceiveModule, RADIO_FRAME_SIZE},
};

pub use crate::protocol::{GetConfigResponse, GetInfoResponse};

pub const DEFAULT_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(6);

pub struct RadioClient {
    module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
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

    pub fn module_receive(&self) -> broadcast::Receiver<Box<ReceiveModule>> {
        self.module_rx_send.subscribe()
    }

    pub async fn transmit(
        &mut self,
        module: usize,
        frame: &Frame<RADIO_FRAME_SIZE>,
    ) -> Result<(), ControllerError> {
        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::TransmitModuleRequest(
                        crate::protocol::TransmitModule {
                            module,
                            frame: RadioFrame::new_from_frame(frame),
                        },
                    ))
                    .build()
                    .into(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        Ok(())
    }

    pub async fn set_modulation(
        &mut self,
        module: usize,
        modulation: Modulation,
    ) -> Result<(), ControllerError> {
        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::SetModulationRequest(
                        crate::protocol::SetModulationRequest { module, modulation },
                    ))
                    .build()
                    .into(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        Ok(())
    }

    pub async fn set_radio_config(
        &mut self,
        module: usize,
        config: RadioConfig,
    ) -> Result<(), ControllerError> {
        self.client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::SetRadioConfigRequest(
                        crate::protocol::SetRadioConfigRequest { module, config },
                    ))
                    .build()
                    .into(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        Ok(())
    }

    async fn listen_rx(
        mut rx_recv: broadcast::Receiver<Box<Message>>,
        module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                Ok(message) = rx_recv.recv() => {
                    match message.payload {
                        Payload::ReceiveModule(rx) => {
                            let _ = module_rx_send.send(Box::new(rx));
                        },
                        _ => {}
                    }
                }
                _ = cancel.cancelled() => {

                }
            }
        }
    }

    pub async fn get_info(&mut self) -> Result<GetInfoResponse, ControllerError> {
        let response = self
            .client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::GetInfoRequest)
                    .build(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        match response.payload {
            Payload::GetInfoResponse(info) => Ok(info),
            _ => Err(ControllerError::DecodeError),
        }
    }

    pub async fn get_config(&mut self) -> Result<GetConfigResponse, ControllerError> {
        let response = self
            .client
            .request(
                MessageBuilder::new()
                    .with_id(self.client.gen_id())
                    .with_payload(Payload::GetConfigRequest)
                    .build(),
                DEFAULT_TIMEOUT,
            )
            .await?;

        match response.payload {
            Payload::GetConfigResponse(config) => Ok(config),
            _ => Err(ControllerError::DecodeError),
        }
    }

    pub fn cancel(&mut self) {
        self.client.cancel();
        self.cancel.cancel();
    }
}

use std::{sync::Arc, time::Instant};

use kaonic_ctrl::{
    protocol::{Message, MessageBuilder, Payload, RadioFrame, ReceiveModule},
    server::{Server, ServerHandler},
};
use kaonic_radio::{
    error::KaonicError,
    platform::{
        create_machine, kaonic1s::Kaonic1SRadioEvent, PlatformRadio, PlatformRadioEvent,
        PlatformRadioFrame,
    },
    radio::Radio,
};

use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio_util::sync::CancellationToken;

pub type SharedRadio = Arc<std::sync::Mutex<PlatformRadio>>;

pub struct RadioServer {
    radios: Vec<SharedRadio>,
    cancel: CancellationToken,
}

impl RadioServer {
    pub fn new(
        client_send: mpsc::Sender<Box<Message>>,
        cancel: CancellationToken,
    ) -> Result<Self, KaonicError> {
        let mut machine = create_machine()?;

        let (module_rx_send, module_rx_recv) = broadcast::channel(16);

        let mut radio_index = 0;
        let mut radios = Vec::new();
        loop {
            let radio = machine.take_radio(radio_index);
            if radio.is_none() {
                break;
            }

            log::debug!("setup radio[{}]", radio_index);

            let (event_send, event_recv) = watch::channel(false);

            let radio = radio.unwrap();
            let event = radio.event();

            let radio = Arc::new(std::sync::Mutex::new(radio));

            std::thread::Builder::new()
                .name(format!("kaonic-radio-event-{}", radio_index))
                .spawn(move || {
                    radio_event_thread(event, event_send);
                })
                .unwrap();

            {
                let cancel = cancel.clone();
                let module_rx_send = module_rx_send.clone();
                let radio = radio.clone();

                tokio::spawn(Box::pin(async move {
                    Self::manage_radio(
                        radio_index as u16,
                        radio,
                        module_rx_send,
                        event_recv,
                        cancel,
                    )
                    .await;
                }));
            }

            radio_index += 1;
            radios.push(radio);

            break;
        }

        {
            let cancel = cancel.clone();
            tokio::spawn(Box::pin(async move {
                let _ = Self::manage_module_receive(client_send, module_rx_recv, cancel).await;
            }));
        }

        Ok(Self { radios, cancel })
    }

    async fn manage_module_receive(
        client_send: mpsc::Sender<Box<Message>>,
        mut module_rx_recv: broadcast::Receiver<Box<ReceiveModule>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;

                Ok(rx) = module_rx_recv.recv() => {
                    let _ = client_send.send(Box::new(MessageBuilder::new()
                        .with_rnd_id(OsRng)
                        .with_payload(Payload::ReceiveModule(*rx))
                        .build())).await;
                },

                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn manage_radio(
        module: u16,
        radio: SharedRadio,
        module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
        mut event_recv: watch::Receiver<bool>,
        cancel: CancellationToken,
    ) {
        let mut rx_frame = PlatformRadioFrame::new();

        loop {
            let mut receive_module = Box::new(ReceiveModule::new());

            tokio::select! {
                biased;

                _ = event_recv.changed() => {

                    let _ = radio.lock().unwrap().update_event();

                    match radio.lock().unwrap()
                        .receive(rx_frame.clear(), core::time::Duration::from_millis(2))
                    {
                        Ok(_rr) => {
                            receive_module.module = module.into();
                            receive_module.frame = RadioFrame::new_from_frame(&rx_frame);

                            if let Err(_) = module_rx_send.send(receive_module) {
                                log::error!("can't send module-rx event");
                            }
                        }
                        Err(_) => {}
                    }
                },

                _ = cancel.cancelled() => {
                    break;
                }
            }
        }
    }
}

impl ServerHandler<Message> for RadioServer {
    fn handle_message(
        &mut self,
        request: &Message,
        mut response: Box<Message>,
    ) -> Option<Box<Message>> {
        let start_time = Instant::now();

        *response.as_mut() = *request;

        match request.payload {
            Payload::TransmitModuleRequest(tx) => {
                if tx.module < self.radios.len() {
                    let mut radio = self.radios[tx.module].lock().unwrap();

                    radio.transmit(&PlatformRadioFrame::new_from_slice(tx.frame.as_slice()));

                    response.payload = Payload::TransmitModuleResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetRadioConfigRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module]
                        .lock()
                        .unwrap()
                        .configure(&set.config);

                    response.payload = Payload::SetRadioConfigResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetModulationRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module]
                        .lock()
                        .unwrap()
                        .set_modulation(&set.modulation);

                    response.payload = Payload::SetModulationResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetInfoRequest => {
                response.payload = Payload::GetInfoResponse(kaonic_ctrl::protocol::GetInfoResponse {
                    module_count: self.radios.len(),
                });
            }
            Payload::Ping => {
                response.payload = Payload::Pong;
            }
            _ => {}
        }

        log::trace!("request took {} usec", start_time.elapsed().as_micros());

        Some(response)
    }

    fn new_message(&mut self) -> Box<Message> {
        Box::new(Message::new())
    }
}

fn radio_event_thread(
    event: Arc<std::sync::Mutex<PlatformRadioEvent>>,
    notify: tokio::sync::watch::Sender<bool>,
) {
    loop {
        if event.lock().unwrap().wait_for_event(None) {
            let _ = notify.send(true);
        }
    }
}

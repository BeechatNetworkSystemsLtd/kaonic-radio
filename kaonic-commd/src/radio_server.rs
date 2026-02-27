use std::{sync::Arc, time::Instant};

use kaonic_ctrl::{
    protocol::{Message, MessageBuilder, Payload, RadioFrame, ReceiveModule},
    server::{Server, ServerRequest},
};
use kaonic_radio::{
    error::KaonicError,
    platform::{create_machine, kaonic1s::Kaonic1SRadioEvent, PlatformRadio, PlatformRadioFrame},
    radio::Radio,
};
use rand::rngs::OsRng;
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

pub struct RadioServer {
    server: Server<Message>,
    module_rx_recv: mpsc::Receiver<ReceiveModule>,
    req_recv: broadcast::Receiver<ServerRequest<Message>>,
    radios: Vec<Arc<Mutex<PlatformRadio>>>,
    cancel: CancellationToken,
}

impl RadioServer {
    pub fn new(
        server: Server<Message>,
        req_recv: broadcast::Receiver<ServerRequest<Message>>,
        cancel: CancellationToken,
    ) -> Result<Self, KaonicError> {
        let mut machine = create_machine()?;

        let (module_rx_send, module_rx_recv) = mpsc::channel(16);

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

            let radio = Arc::new(Mutex::new(radio));

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

        Ok(Self {
            server,
            radios,
            req_recv,
            module_rx_recv,
            cancel,
        })
    }

    pub async fn serve(mut self) {
        let mut res_message = MessageBuilder::new()
            .with_id(0)
            .with_payload(Payload::NotImplemented)
            .build();

        loop {
            tokio::select! {
                biased;
                Ok(request) = self.req_recv.recv() => {

                    log::trace!("new request {}us", request.time().elapsed().as_micros());

                    res_message.id = request.message().id;

                    self.handle_request(request, &mut res_message).await;
                }
                Some(rx) = self.module_rx_recv.recv() => {
                   self.server.broadcast(
                        MessageBuilder::new()
                            .with_rnd_id(OsRng)
                            .with_payload(Payload::ReceiveModule(rx))
                            .build().into())
                    .await;
                }
                _ = self.cancel.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn handle_request(&mut self, request: ServerRequest<Message>, res_message: &mut Message) {
        let req_message = request.message();

        let req_time = request.time();
        let start_time = Instant::now();

        match req_message.payload {
            Payload::TransmitModuleRequest(tx) => {
                if tx.module < self.radios.len() {
                    let _ = self.radios[tx.module]
                        .lock()
                        .await
                        .transmit(&PlatformRadioFrame::new_from_slice(tx.frame.as_slice()));

                    res_message.payload = Payload::TransmitModuleResponse;
                } else {
                    res_message.payload = Payload::Error;
                }
            }
            Payload::SetRadioConfigRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module].lock().await.configure(&set.config);

                    res_message.payload = Payload::SetRadioConfigResponse;
                } else {
                    res_message.payload = Payload::Error;
                }
            }
            Payload::SetModulationRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module]
                        .lock()
                        .await
                        .set_modulation(&set.modulation);

                    res_message.payload = Payload::SetModulationResponse;
                } else {
                    res_message.payload = Payload::Error;
                }
            }
            Payload::GetInfoRequest => {}
            Payload::Ping => {
                res_message.payload = Payload::Pong;
            }
            _ => {}
        }

        let _ = request.response(Box::new(*res_message)).await;

        log::trace!(
            "request handled in {} ms total:{} usec",
            start_time.elapsed().as_millis(),
            req_time.elapsed().as_micros(),
        );
    }

    async fn manage_radio(
        module: u16,
        radio: Arc<Mutex<PlatformRadio>>,
        module_rx_send: mpsc::Sender<ReceiveModule>,
        mut event_recv: watch::Receiver<bool>,
        cancel: CancellationToken,
    ) {
        let mut rx_frame = PlatformRadioFrame::new();

        loop {
            tokio::select! {
                biased;

                _ = event_recv.changed() => {

                    let _ = radio.lock().await.update_event();

                    match radio.lock().await
                        .receive(rx_frame.clear(), core::time::Duration::from_millis(2))
                    {
                        Ok(_rr) => {
                            if let Err(_) = module_rx_send.send(
                                ReceiveModule {
                                    module: module.into(),
                                    frame: RadioFrame::new_from_frame(&rx_frame),
                                }).await {
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

fn radio_event_thread(
    event: Arc<std::sync::Mutex<Kaonic1SRadioEvent>>,
    notify: tokio::sync::watch::Sender<bool>,
) {
    loop {
        if event.lock().unwrap().wait_for_event(None) {
            let _ = notify.send(true);
        }
    }
}

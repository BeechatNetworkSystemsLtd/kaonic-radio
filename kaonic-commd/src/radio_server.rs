use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use kaonic_ctrl::{
    protocol::{
        GetStatisticsResponse, Message, MessageBuilder, Payload, RadioFrame, ReceiveModule,
        TransmitModule,
    },
    server::ServerHandler,
};
use kaonic_radio::{
    error::KaonicError,
    platform::{PlatformRadio, PlatformRadioEvent, PlatformRadioFrame, create_machine},
    radio::Radio,
};

use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

pub type SharedRadio = Arc<std::sync::Mutex<PlatformRadio>>;

#[derive(Default)]
pub struct ModuleStats {
    pub rx_packets: AtomicU64,
    pub tx_packets: AtomicU64,
    pub rx_bytes: AtomicU64,
    pub tx_bytes: AtomicU64,
    pub rx_errors: AtomicU64,
    pub tx_errors: AtomicU64,
}

pub type SharedModuleStats = Arc<ModuleStats>;

pub struct RadioServer {
    radios: Vec<SharedRadio>,
    stats: Vec<SharedModuleStats>,
    module_rx_send: broadcast::Sender<Box<ReceiveModule>>,
    module_tx_send: broadcast::Sender<Box<TransmitModule>>,
    cancel: CancellationToken,
    serial: String,
    mtu: usize,
}

impl RadioServer {
    pub fn new(
        client_send: mpsc::Sender<Box<Message>>,
        cancel: CancellationToken,
        serial: String,
        mtu: usize,
    ) -> Result<Self, KaonicError> {
        let mut machine = create_machine()?;

        let (module_rx_send, module_rx_recv) = broadcast::channel(16);
        let (module_tx_send, module_tx_recv) = broadcast::channel(16);

        let mut radio_index = 0;
        let mut radios = Vec::new();
        let mut stats: Vec<SharedModuleStats> = Vec::new();
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
            let module_stats: SharedModuleStats = Arc::new(ModuleStats::default());

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
                let module_stats = module_stats.clone();

                tokio::spawn(Box::pin(async move {
                    Self::manage_radio(
                        radio_index as u16,
                        radio,
                        module_rx_send,
                        event_recv,
                        cancel,
                        module_stats,
                    )
                    .await;
                }));
            }

            radio_index += 1;
            radios.push(radio);
            stats.push(module_stats);
        }

        {
            let cancel = cancel.clone();
            let client_send = client_send.clone();
            tokio::spawn(Box::pin(async move {
                let _ = Self::manage_module_receive(client_send, module_rx_recv, cancel).await;
            }));
        }

        {
            let cancel = cancel.clone();
            let client_send = client_send.clone();
            tokio::spawn(Box::pin(async move {
                let _ = Self::manage_module_transmit(client_send, module_tx_recv, cancel).await;
            }));
        }

        Ok(Self {
            radios,
            stats,
            module_rx_send,
            module_tx_send,
            cancel,
            serial,
            mtu,
        })
    }

    /// Returns clones of the shared radio handles.
    pub fn radios(&self) -> Vec<SharedRadio> {
        self.radios.clone()
    }

    /// Returns the number of available radio modules.
    pub fn module_count(&self) -> usize {
        self.radios.len()
    }

    /// Returns clones of the per-module statistics handles.
    pub fn stats(&self) -> Vec<SharedModuleStats> {
        self.stats.clone()
    }

    /// Subscribes to the broadcast channel of received radio frames.
    pub fn subscribe_rx(&self) -> broadcast::Receiver<Box<ReceiveModule>> {
        self.module_rx_send.subscribe()
    }

    /// Returns a clone of the broadcast sender for received radio frames.
    pub fn rx_sender(&self) -> broadcast::Sender<Box<ReceiveModule>> {
        self.module_rx_send.clone()
    }

    /// Returns a clone of the broadcast sender for transmitted radio frames.
    pub fn tx_sender(&self) -> broadcast::Sender<Box<TransmitModule>> {
        self.module_tx_send.clone()
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

    async fn manage_module_transmit(
        client_send: mpsc::Sender<Box<Message>>,
        mut module_tx_recv: broadcast::Receiver<Box<TransmitModule>>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;


                Ok(tx) = module_tx_recv.recv() => {
                    if false {
                        let _ = client_send.send(Box::new(MessageBuilder::new()
                            .with_rnd_id(OsRng)
                            .with_payload(Payload::TransmitModuleEvent(*tx))
                            .build())).await;
                    }
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
        stats: SharedModuleStats,
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
                        Ok(rr) => {
                            let frame_len = rx_frame.len() as u64;
                            stats.rx_packets.fetch_add(1, Ordering::Relaxed);
                            stats.rx_bytes.fetch_add(frame_len, Ordering::Relaxed);

                            receive_module.module = module.into();
                            receive_module.frame = RadioFrame::new_from_frame(&rx_frame);
                            receive_module.rssi = rr.rssi;

                            if let Err(_) = module_rx_send.send(receive_module) {
                                log::error!("can't send module-rx event");
                            }
                        }
                        Err(e) => {
                            if e != KaonicError::Timeout {
                                stats.rx_errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
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

        *response.as_mut() = request.clone();

        match request.payload {
            Payload::TransmitModuleRequest(tx) => {
                if tx.module < self.radios.len() {
                    let mut radio = self.radios[tx.module].lock().unwrap();
                    let frame_len = tx.frame.as_slice().len() as u64;

                    if let Ok(_) =
                        radio.transmit(&PlatformRadioFrame::new_from_slice(tx.frame.as_slice()))
                    {
                        self.stats[tx.module]
                            .tx_packets
                            .fetch_add(1, Ordering::Relaxed);
                        self.stats[tx.module]
                            .tx_bytes
                            .fetch_add(frame_len, Ordering::Relaxed);
                        let _ = self.module_tx_send.send(Box::new(tx));
                        response.payload = Payload::TransmitModuleResponse;
                    } else {
                        self.stats[tx.module]
                            .tx_errors
                            .fetch_add(1, Ordering::Relaxed);
                        response.payload = Payload::Error;
                    }
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetRadioConfigRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module]
                        .lock()
                        .unwrap()
                        .set_config(&set.config);

                    response.payload = Payload::SetRadioConfigResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetRadioConfigRequest(get) => {
                if get.module < self.radios.len() {
                    let config = self.radios[get.module].lock().unwrap().get_config();

                    response.payload = Payload::GetRadioConfigResponse(
                        kaonic_ctrl::protocol::GetRadioConfigResponse {
                            module: get.module,
                            config,
                        },
                    );
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
            Payload::GetModulationRequest(get) => {
                if get.module < self.radios.len() {
                    let modulation = self.radios[get.module].lock().unwrap().get_modulation();

                    response.payload = Payload::GetModulationResponse(
                        kaonic_ctrl::protocol::GetModulationResponse {
                            module: get.module,
                            modulation,
                        },
                    );
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetInfoRequest => {
                response.payload =
                    Payload::GetInfoResponse(kaonic_ctrl::protocol::GetInfoResponse {
                        module_count: self.radios.len(),
                        serial: self.serial.clone(),
                        mtu: self.mtu,
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    });
            }
            Payload::GetStatisticsRequest(req) => {
                if req.module < self.stats.len() {
                    let s = &self.stats[req.module];
                    response.payload = Payload::GetStatisticsResponse(GetStatisticsResponse {
                        module: req.module,
                        rx_packets: s.rx_packets.load(Ordering::Relaxed),
                        tx_packets: s.tx_packets.load(Ordering::Relaxed),
                        rx_bytes: s.rx_bytes.load(Ordering::Relaxed),
                        tx_bytes: s.tx_bytes.load(Ordering::Relaxed),
                        rx_errors: s.rx_errors.load(Ordering::Relaxed),
                        tx_errors: s.tx_errors.load(Ordering::Relaxed),
                    });
                } else {
                    response.payload = Payload::Error;
                }
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

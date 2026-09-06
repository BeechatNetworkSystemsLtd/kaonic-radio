use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use kaonic_ctrl::{
    protocol::{
        GetStatisticsResponse, Message, MessageBuilder, Payload, ReceiveModule, TransmitModule,
    },
    server::ServerHandler,
};
use kaonic_radio::{
    error::KaonicError,
    platform::{PlatformRadioEvent, PlatformRadioFrame, create_machine},
};

use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::radio_worker::{self, RadioHandle, WorkerSignal};

const MODULE_EVENT_CHANNEL_CAPACITY: usize = 256;

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
    radios: Vec<RadioHandle>,
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

        let (module_rx_send, module_rx_recv) = broadcast::channel(MODULE_EVENT_CHANNEL_CAPACITY);
        let (module_tx_send, module_tx_recv) = broadcast::channel(MODULE_EVENT_CHANNEL_CAPACITY);

        let mut radio_index = 0;
        let mut radios = Vec::new();
        let mut stats: Vec<SharedModuleStats> = Vec::new();
        loop {
            let radio = machine.take_radio(radio_index);
            if radio.is_none() {
                break;
            }

            log::debug!("setup radio[{}]", radio_index);

            let radio = radio.unwrap();
            let event = radio.event();

            let module_stats: SharedModuleStats = Arc::new(ModuleStats::default());
            let signal = WorkerSignal::new();

            {
                let signal = signal.clone();
                std::thread::Builder::new()
                    .name(format!("kaonic-radio-event-{}", radio_index))
                    .spawn(move || {
                        radio_event_thread(event, signal);
                    })
                    .unwrap();
            }

            let handle = radio_worker::spawn(
                radio_index,
                radio,
                signal,
                module_rx_send.clone(),
                module_stats.clone(),
            );

            radio_index += 1;
            radios.push(handle);
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

    /// Returns clones of the radio worker handles.
    pub fn radios(&self) -> Vec<RadioHandle> {
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

                recv_result = module_rx_recv.recv() => match recv_result {
                    Ok(rx) => {
                        let _ = client_send.send(Box::new(MessageBuilder::new()
                            .with_rnd_id(OsRng)
                            .with_payload(Payload::ReceiveModule(*rx))
                            .build())).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("radio server rx stream lagged by {skipped} messages");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
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


                recv_result = module_tx_recv.recv() => match recv_result {
                    Ok(tx) => {
                        if false {
                            let _ = client_send.send(Box::new(MessageBuilder::new()
                                .with_rnd_id(OsRng)
                                .with_payload(Payload::TransmitModuleEvent(*tx))
                                .build())).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("radio server tx stream lagged by {skipped} messages");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
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
                    let frame = PlatformRadioFrame::new_from_slice(tx.frame.as_slice());

                    if let Ok(_) = self.radios[tx.module].transmit(frame) {
                        let _ = self.module_tx_send.send(Box::new(tx));
                        response.payload = Payload::TransmitModuleResponse;
                    } else {
                        response.payload = Payload::Error;
                    }
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetRadioConfigRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module].set_config(set.config);

                    response.payload = Payload::SetRadioConfigResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetRadioConfigRequest(get) => {
                if get.module < self.radios.len() {
                    match self.radios[get.module].get_config() {
                        Ok(config) => {
                            response.payload = Payload::GetRadioConfigResponse(
                                kaonic_ctrl::protocol::GetRadioConfigResponse {
                                    module: get.module,
                                    config,
                                },
                            );
                        }
                        Err(_) => response.payload = Payload::Error,
                    }
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetModulationRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module].set_modulation(set.modulation);

                    response.payload = Payload::SetModulationResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetModulationRequest(get) => {
                if get.module < self.radios.len() {
                    match self.radios[get.module].get_modulation() {
                        Ok(modulation) => {
                            response.payload = Payload::GetModulationResponse(
                                kaonic_ctrl::protocol::GetModulationResponse {
                                    module: get.module,
                                    modulation,
                                },
                            );
                        }
                        Err(_) => response.payload = Payload::Error,
                    }
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetAccelerationRequest(set) => {
                if set.module < self.radios.len() {
                    let _ = self.radios[set.module].set_accelerator(set.acceleration);

                    response.payload = Payload::SetAccelerationResponse;
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetAccelerationRequest(get) => {
                if get.module < self.radios.len() {
                    match self.radios[get.module].get_accelerator() {
                        Ok(acceleration) => {
                            response.payload = Payload::GetAccelerationResponse(
                                kaonic_ctrl::protocol::GetAccelerationResponse {
                                    module: get.module,
                                    acceleration,
                                },
                            );
                        }
                        Err(_) => response.payload = Payload::Error,
                    }
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::SetAntennaRequest(set) => {
                if set.module < self.radios.len() {
                    response.payload = match self.radios[set.module].set_antenna(set.band, set.antenna)
                    {
                        Ok(_) => Payload::SetAntennaResponse,
                        Err(_) => Payload::Error,
                    };
                } else {
                    response.payload = Payload::Error;
                }
            }
            Payload::GetAntennaRequest(get) => {
                if get.module < self.radios.len() {
                    match self.radios[get.module].get_antenna(get.band) {
                        Ok(antenna) => {
                            response.payload = Payload::GetAntennaResponse(
                                kaonic_ctrl::protocol::GetAntennaResponse {
                                    module: get.module,
                                    band: get.band,
                                    antenna,
                                },
                            );
                        }
                        Err(_) => response.payload = Payload::Error,
                    }
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
            Payload::TransmitBatchRequest(ref batch) => {
                if batch.module < self.radios.len() {
                    let frames: Vec<PlatformRadioFrame> = batch
                        .frames
                        .iter()
                        .map(|f| PlatformRadioFrame::new_from_slice(&f.data))
                        .collect();

                    let (sent, errors) = self.radios[batch.module].transmit_batch(frames);

                    response.payload = Payload::TransmitBatchResponse(
                        kaonic_ctrl::protocol::TransmitBatchResponse {
                            module: batch.module,
                            sent,
                            errors,
                        },
                    );
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

/// Puts the calling thread on SCHED_FIFO so it preempts normal (EEVDF)
/// threads immediately on wakeup. Requires CAP_SYS_NICE.
pub(crate) fn set_realtime_priority(priority: i32) -> bool {
    let param = libc::sched_param {
        sched_priority: priority,
    };

    unsafe { libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param) == 0 }
}

fn radio_event_thread(
    event: Arc<std::sync::Mutex<PlatformRadioEvent>>,
    signal: Arc<WorkerSignal>,
) {
    // GPIO IRQ servicing must not wait out a timeslice behind CPU-bound
    // work (gRPC, UI); modest RT priority keeps edge-to-notify latency
    // in the microsecond range. Priority stays below kernel irq threads (50).
    if !set_realtime_priority(10) {
        log::warn!("radio event thread: can't set SCHED_FIFO priority");
    }

    loop {
        if event.lock().unwrap().wait_for_event(None) {
            signal.notify();
        }
    }
}

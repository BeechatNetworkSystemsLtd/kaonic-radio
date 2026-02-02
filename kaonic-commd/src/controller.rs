use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use kaonic_net::{
    muxer::CurrentTime,
    network::Network,
    packet::{LdpcPacketCoder, PacketCoder},
};
use kaonic_radio::{
    error::KaonicError,
    frame::{Frame, FrameSegment},
    modulation::Modulation,
    platform::{create_machine, kaonic1s::Kaonic1SRadioEvent, PlatformRadio},
    radio::{Radio, RadioConfig},
};
use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::task::block_in_place;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MAX_SEGMENTS_COUNT: usize = 6;

type Coder = LdpcPacketCoder<2048>;

pub type RadioFrame = Frame<2048>;
pub type NetworkFrame = FrameSegment<2048, MAX_SEGMENTS_COUNT>;

type RadioNetwork =
    Network<2048, MAX_SEGMENTS_COUNT, 12, { Coder::MAX_PAYLOAD_SIZE }, LdpcPacketCoder<2048>>;

#[derive(Clone, Copy)]
pub struct NetworkReceive {
    pub frame: NetworkFrame,
}

#[derive(Clone, Copy)]
pub struct NetworkTransmit {
    pub frame: NetworkFrame,
}

#[derive(Clone, Copy, Debug)]
pub struct ModuleReceive {
    pub module: usize,
    pub frame: RadioFrame,
    pub rssi: i8,
}

#[derive(Clone, Copy)]
pub struct ModuleTransmit {
    pub module: usize,
    pub frame: RadioFrame,
}

#[derive(Clone, Copy)]
pub struct ModuleConfig {
    pub module: usize,
    pub config: RadioConfig,
}

#[derive(Clone, Copy)]
pub struct ModuleModulation {
    pub module: usize,
    pub modulation: Modulation,
}

#[derive(Clone, Copy)]
pub enum RadioCommand {
    Transmit(ModuleTransmit),
    Configure(ModuleConfig),
    SetModulation(ModuleModulation),
    Shutdown,
}

pub struct RadioController {
    network_rx_send: broadcast::Sender<NetworkReceive>,
    network_tx_send: broadcast::Sender<NetworkTransmit>,
    module_send: broadcast::Sender<ModuleReceive>,
    command_send: broadcast::Sender<RadioCommand>,
    shutdown: CancellationToken,
    worker_handles: Vec<JoinHandle<()>>,
}

impl RadioController {
    pub fn new(shutdown: CancellationToken) -> Result<Self, KaonicError> {
        let mut machine = create_machine()?;

        // Increase channel capacities to reduce risk of lagging drops under load
        let (module_send, _) = broadcast::channel(256);
        let (command_send, _) = broadcast::channel(128);
        let (network_rx_send, _) = broadcast::channel(128);
        let (network_tx_send, _) = broadcast::channel(128);

        let mut worker_handles: Vec<JoinHandle<()>> = Vec::new();

        // Create the shared network and spawn RX/TX managers first so they
        // subscribe to `module_send` before radio workers start sending.
        let network = Arc::new(Mutex::new(RadioNetwork::new(Coder::new())));

        let rx_handle = tokio::spawn(manage_rx_network(
            network.clone(),
            network_rx_send.clone(),
            module_send.subscribe(),
            shutdown.clone(),
        ));

        let tx_handle = tokio::spawn(manage_tx_network(
            network.clone(),
            network_tx_send.subscribe(),
            command_send.clone(),
            shutdown.clone(),
        ));

        worker_handles.push(rx_handle);
        worker_handles.push(tx_handle);

        let mut radio_index = 0;
        loop {
            let radio = machine.take_radio(radio_index);
            if radio.is_none() {
                break;
            }

            let (event_send, event_recv) = watch::channel(false);

            let radio = radio.unwrap();
            let event = radio.event();

            let radio = Arc::new(std::sync::Mutex::new(radio));

            std::thread::Builder::new()
                .name("radio-event".to_string())
                .spawn(move || {
                    radio_event_thread(event, event_send);
                })
                .unwrap();

            let handle = tokio::spawn(manage_radio(
                radio_index,
                radio,
                command_send.subscribe(),
                module_send.clone(),
                event_recv,
                shutdown.clone(),
            ));

            worker_handles.push(handle);

            radio_index += 1;
        }

        Ok(Self {
            network_rx_send,
            network_tx_send,
            module_send,
            command_send,
            shutdown,
            worker_handles: worker_handles,
        })
    }

    pub async fn wait_for_workers(&self) {
        // for (i, h) in self.worker_handles.into_iter().enumerate() {
        //     log::info!("Waiting for worker {} to finish", i);
        //     let _ = h.await;
        //     log::info!("Worker {} finished", i);
        // }
    }

    pub fn execute(&self, command: RadioCommand) {
        let _ = self.command_send.send(command);
    }

    pub fn network_transmit(&self, frame: NetworkFrame) -> Result<(), KaonicError> {
        log::debug!(
            "RadioController::network_transmit called, frame len={} bytes",
            frame.as_slice().len()
        );
        let _ = self.network_tx_send.send(NetworkTransmit { frame });
        Ok(())
    }

    pub fn network_receive(&self) -> broadcast::Receiver<NetworkReceive> {
        self.network_rx_send.subscribe()
    }

    pub fn module_receive(&self, _module: usize) -> broadcast::Receiver<ModuleReceive> {
        self.module_send.subscribe()
    }

    pub fn command(&self) -> broadcast::Sender<RadioCommand> {
        self.command_send.clone()
    }
}

fn get_current_time() -> CurrentTime {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis()
}

async fn manage_rx_network(
    network: Arc<Mutex<RadioNetwork>>,
    network_send: broadcast::Sender<NetworkReceive>,
    mut module_recv: broadcast::Receiver<ModuleReceive>,
    mut shutdown: CancellationToken,
) {
    loop {
        network.lock().await.process(get_current_time(), |frame| {
            log::debug!("Network RX Frame {}B", frame.len());
            let _ = network_send.send(NetworkReceive {
                frame: FrameSegment::new_from_slice(frame),
            });
        });

        tokio::select! {
            _ = shutdown.cancelled() => {
                log::info!("Network RX manager received shutdown");
                break;
            }
            module_recv = module_recv.recv() => {
                match module_recv {
                    Ok(event) => {
                        let _ = network.lock().await.receive(get_current_time(), &event.frame);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("manage_rx_network: receiver lagged, skipped {} messages", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        log::warn!("manage_rx_network: module_recv channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(core::time::Duration::from_millis(100)) => {

            }
        }
    }
}

async fn manage_tx_network(
    network: Arc<Mutex<RadioNetwork>>,
    mut network_tx_recv: broadcast::Receiver<NetworkTransmit>,
    command_send: broadcast::Sender<RadioCommand>,
    mut shutdown: CancellationToken,
) {
    let mut output_frames = [Frame::new(); MAX_SEGMENTS_COUNT];

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                log::info!("Network TX manager received shutdown");
                break;
            }
            network_tx_recv = network_tx_recv.recv() => {
                match network_tx_recv {
                    Ok(tx_frame) => {
                        log::debug!("manage_tx_network: received network transmit frame len={} bytes", tx_frame.frame.as_slice().len());
                        let _ = network.lock().await.transmit(tx_frame.frame.as_slice(), OsRng, &mut output_frames, |data| {
                            for chunk in data {
                                log::debug!("manage_tx_network: sending fragment to radio module=0 len={}", chunk.len());
                                let _ = command_send.send(RadioCommand::Transmit(ModuleTransmit{
                                    module: 0,
                                    frame: RadioFrame::new_from_slice(chunk),
                                }));
                            }
                            Ok(())
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("manage_tx_network: receiver lagged, skipped {} messages", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        log::warn!("manage_tx_network: network_tx_recv channel closed");
                        break;
                    }
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

async fn manage_radio(
    module: usize,
    radio: Arc<std::sync::Mutex<PlatformRadio>>,
    mut command_recv: broadcast::Receiver<RadioCommand>,
    module_send: broadcast::Sender<ModuleReceive>,
    mut event_recv: watch::Receiver<bool>,
    mut shutdown: CancellationToken,
) {
    let mut rx_frame = RadioFrame::new();

    loop {
        tokio::select! {
            biased;

            _ = event_recv.changed() => {

                let _ = radio.lock().unwrap().update_event();

                match radio.lock().unwrap()
                    .receive(rx_frame.clear(), core::time::Duration::from_millis(2))
                {
                    Ok(rr) => {
                        match module_send.send(ModuleReceive {
                            module,
                            frame: rx_frame,
                            rssi: rr.rssi,
                        }) {
                            Ok(_subs) => {}
                            Err(e) => {
                                log::warn!("manage_radio: module_send error sending: {:?}", e)
                            }
                        }
                    }
                    Err(_) => {}
                }
            },
            cmd = command_recv.recv() => {
                match cmd {
                    Ok(RadioCommand::Transmit(command)) => {
                        if command.module == module {
                            let _ = radio.lock().unwrap().transmit(&command.frame);
                        }
                    }
                    Ok(RadioCommand::Configure(command)) => {
                        if command.module == module {
                            let _ = radio.lock().unwrap().configure(&command.config);
                        }
                    }
                    Ok(RadioCommand::SetModulation(command)) => {
                        if command.module == module {
                            let _ = radio.lock().unwrap().set_modulation(&command.modulation);
                        }
                    }
                    Ok(RadioCommand::Shutdown) => {}
                    Err(_) => {}
                }
            }

            _ = shutdown.cancelled() => {
                log::info!("Radio module {} received cancellation", module);
                break;
            }
        };
    }
}

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use kaonic_net::{
    muxer::CurrentTime,
    network::{self, Network},
    packet::{LdpcPacketCoder, PacketCoder},
};
use kaonic_radio::{
    error::KaonicError,
    frame::{Frame, FrameSegment},
    modulation::Modulation,
    platform::{create_machine, PlatformRadio},
    radio::{self, Radio, RadioConfig},
};
use rand::rngs::OsRng;
use tokio::sync::{self, broadcast, Mutex};
use tokio_util::sync::CancellationToken;

const MAX_SEGMENTS_COUNT: usize = 6;

type Coder = LdpcPacketCoder<2048>;
type RadioFrame = Frame<2048>;
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

#[derive(Clone, Copy)]
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
    network: Arc<Mutex<RadioNetwork>>,
    network_rx_send: broadcast::Sender<NetworkReceive>,
    network_tx_send: broadcast::Sender<NetworkFrame>,
    module_send: broadcast::Sender<ModuleReceive>,
    command_send: broadcast::Sender<RadioCommand>,
}

impl RadioController {
    pub fn new() -> Result<Self, KaonicError> {
        let mut machine = create_machine()?;

        let (module_send, _) = broadcast::channel(8);
        let (command_send, _) = broadcast::channel(8);
        let (network_rx_send, _) = broadcast::channel(8);
        let (network_tx_send, _) = broadcast::channel(8);

        let mut radio_index = 0;
        loop {
            let mut radio = machine.take_radio(radio_index);
            if radio.is_none() {
                break;
            }

            let radio = Arc::new(Mutex::new(radio.unwrap()));

            tokio::spawn(manage_radio(
                radio_index,
                radio,
                command_send.subscribe(),
                module_send.clone(),
            ));

            radio_index += 1;
        }

        let network = Arc::new(Mutex::new(RadioNetwork::new(Coder::new())));

        tokio::spawn(manage_rx_network(
            network.clone(),
            network_rx_send.clone(),
            module_send.subscribe(),
        ));

        tokio::spawn(manage_tx_network(
            network.clone(),
            network_tx_send.subscribe(),
            command_send.clone(),
        ));

        Ok(Self {
            network,
            network_rx_send,
            network_tx_send,
            module_send,
            command_send,
        })
    }

    pub fn execute(&self, command: RadioCommand) {
        let _ = self.command_send.send(command);
    }

    pub fn network_transmit(&self, frame: NetworkFrame) -> Result<(), KaonicError> {
        self.network_tx_send.send(frame);
        Ok(())
    }

    pub fn module_receive(&self, _module: usize) -> broadcast::Receiver<ModuleReceive> {
        self.module_send.subscribe()
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
) {
    loop {
        tokio::select! {
            Ok(event) = module_recv.recv() => {
                let _ = network.lock().await.receive(get_current_time(), &event.frame);

                network.lock().await.process(get_current_time(), | frame | {
                    let _ = network_send.send(NetworkReceive {
                                frame: FrameSegment::new_from_slice(frame),
                            });
                });

            }
        }
    }
}

async fn manage_tx_network(
    network: Arc<Mutex<RadioNetwork>>,
    mut network_tx_recv: broadcast::Receiver<NetworkFrame>,
    command_send: broadcast::Sender<RadioCommand>,
) {
    let mut output_frames = [Frame::new(); MAX_SEGMENTS_COUNT];

    loop {
        tokio::select! {
            Ok(tx_frame) = network_tx_recv.recv() => {
                let _ = network.lock().await.transmit(tx_frame.as_slice(), OsRng, &mut output_frames, |data| {
                        for chunk in data {
                            command_send.send(RadioCommand::Transmit(ModuleTransmit{
                                module: 0,
                                frame: RadioFrame::new_from_slice(chunk),
                            }));
                        }
                    Ok(())
                });
            }
        }
    }
}

async fn manage_radio(
    module: usize,
    radio: Arc<Mutex<PlatformRadio>>,
    mut command_recv: broadcast::Receiver<RadioCommand>,
    module_send: broadcast::Sender<ModuleReceive>,
) {
    let mut radio = radio.lock().await;

    let mut rx_frame = RadioFrame::new();

    loop {
        match command_recv.try_recv() {
            Ok(RadioCommand::Transmit(command)) => {
                if command.module == module {
                    let _ = radio.transmit(&command.frame);
                }
            }
            Ok(RadioCommand::Configure(command)) => {
                if command.module == module {
                    let _ = radio.configure(&command.config);
                }
            }
            Ok(RadioCommand::SetModulation(command)) => {
                if command.module == module {
                    let _ = radio.set_modulation(&command.modulation);
                }
            }
            Ok(RadioCommand::Shutdown) => {}
            Err(_) => {}
        }

        match radio.receive(&mut rx_frame, core::time::Duration::from_millis(20)) {
            Ok(rr) => {
                let _ = module_send.send(ModuleReceive {
                    module,
                    frame: rx_frame,
                    rssi: rr.rssi,
                });
            }
            Err(_) => {}
        }
    }
}

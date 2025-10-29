use std::sync::Arc;

use kaonic_radio::platform::kaonic1s::Kaonic1SFrame;
use tokio::sync::{broadcast, oneshot};

#[derive(Clone, Debug)]
pub struct ReceiveEvent {
    pub module: usize,
    pub frame: Kaonic1SFrame,
    pub rssi: i8,
    pub latency_ms: u32,
}

pub struct RadioService {
    workers: Vec<Option<WorkerHandle>>, // index is radio module
}

struct WorkerHandle {
    cmd_tx: std::sync::mpsc::Sender<RadioCommand>,
    rx_tx: broadcast::Sender<ReceiveEvent>,
    #[allow(dead_code)]
    join: std::thread::JoinHandle<()>,
}

enum RadioCommand {
    Configure(
        kaonic_radio::radio::RadioConfig,
        oneshot::Sender<Result<(), String>>,
    ),
    Transmit(Kaonic1SFrame, oneshot::Sender<Result<u32, String>>),
    Shutdown,
}

impl RadioService {
    pub fn new() -> Result<Arc<Self>, String> {
        let mut workers: Vec<Option<WorkerHandle>> = Vec::new();

        // We attempt to create the machine and spawn a worker per available radio
        #[cfg(target_os = "linux")]
        {
            let mut machine = kaonic_radio::platform::create_machine()
                .map_err(|e| format!("Failed to create machine: {:?}", e))?;

            for module in 0..2usize {
                if let Some(radio) = machine.take_radio(module) {
                    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<RadioCommand>();
                    let (rx_tx, _rx_rx) = broadcast::channel::<ReceiveEvent>(64);

                    let rx_tx_clone = rx_tx.clone();

                    let join = std::thread::spawn(move || {
                        run_worker(module, radio, cmd_rx, rx_tx_clone);
                    });

                    workers.push(Some(WorkerHandle {
                        cmd_tx,
                        rx_tx,
                        join,
                    }));
                } else {
                    workers.push(None);
                }
            }
        }

        // Non-Linux fallback: create empty worker slots so APIs still work
        #[cfg(not(target_os = "linux"))]
        {
            for _module in 0..2usize {
                let (rx_tx, _rx_rx) = broadcast::channel::<ReceiveEvent>(64);
                // No worker thread in non-linux mode, but keep placeholder handle with a dummy sender
                let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel::<RadioCommand>();
                let dummy_join = std::thread::spawn(|| {});
                workers.push(Some(WorkerHandle {
                    cmd_tx,
                    rx_tx,
                    join: dummy_join,
                }));
            }
        }

        Ok(Arc::new(Self { workers }))
    }

    fn get_worker(&self, module: usize) -> Result<&WorkerHandle, String> {
        self.workers
            .get(module)
            .and_then(|w| w.as_ref())
            .ok_or_else(|| format!("Radio module {} not available", module))
    }

    pub async fn configure(
        &self,
        module: usize,
        config: kaonic_radio::radio::RadioConfig,
    ) -> Result<(), String> {
        let handle = self.get_worker(module)?;
        let (tx, rx) = oneshot::channel();
        handle
            .cmd_tx
            .send(RadioCommand::Configure(config, tx))
            .map_err(|_| "Worker thread stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("Worker dropped".into()))
    }

    pub async fn transmit(&self, module: usize, frame: &Kaonic1SFrame) -> Result<u32, String> {
        let handle = self.get_worker(module)?;
        let (tx, rx) = oneshot::channel();
        handle
            .cmd_tx
            .send(RadioCommand::Transmit(frame.clone(), tx))
            .map_err(|_| "Worker thread stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("Worker dropped".into()))
    }

    pub fn subscribe(&self, module: usize) -> Result<broadcast::Receiver<ReceiveEvent>, String> {
        let handle = self.get_worker(module)?;
        Ok(handle.rx_tx.subscribe())
    }
}

fn run_worker(
    module: usize,
    mut radio: kaonic_radio::platform::kaonic1s::Kaonic1SRadio,
    cmd_rx: std::sync::mpsc::Receiver<RadioCommand>,
    rx_tx: broadcast::Sender<ReceiveEvent>,
) {
    use kaonic_radio::platform::kaonic1s::FRAME_SIZE;
    use kaonic_radio::{frame::Frame, radio::Radio as _};
    use std::sync::mpsc::TryRecvError;
    use std::time::{Duration, Instant};

    let mut rx_frame = Frame::<FRAME_SIZE>::new();

    loop {
        // Drain any available commands before attempting receive
        match cmd_rx.try_recv() {
            Ok(RadioCommand::Configure(cfg, ack)) => {
                let res = radio
                    .configure(&cfg)
                    .map_err(|e| format!("configure failed: {:?}", e));
                let _ = ack.send(res);
                continue;
            }
            Ok(RadioCommand::Transmit(frame, ack)) => {
                let start = Instant::now();
                let res = radio
                    .transmit(&frame)
                    .map(|_| start.elapsed().as_millis() as u32)
                    .map_err(|e| format!("transmit failed: {:?}", e));
                let _ = ack.send(res);
                continue;
            }
            Ok(RadioCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                break;
            }
            Err(TryRecvError::Empty) => {
                // No command, fall through to receive
            }
        }

        // Idle -> attempt a receive with short timeout
        let start = Instant::now();
        match radio.receive(&mut rx_frame, Duration::from_millis(20)) {
            Ok(rr) => {
                let evt = ReceiveEvent {
                    module,
                    frame: rx_frame,
                    rssi: rr.rssi,
                    latency_ms: start.elapsed().as_millis() as u32,
                };
                let _ = rx_tx.send(evt);
            }
            Err(_e) => {
                // Timeout or hw error during idle receive; ignore and continue
            }
        }
    }
}

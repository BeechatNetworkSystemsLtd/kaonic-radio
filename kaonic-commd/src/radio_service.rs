use std::sync::Arc;

use kaonic_radio::platform::kaonic1s::Kaonic1SFrame;
use kaonic_qos::{ModulationScheme, QoSManager};
use tokio::sync::{broadcast, oneshot};

#[derive(Clone, Debug)]
pub struct ReceiveEvent {
    pub module: usize,
    pub frame: Kaonic1SFrame,
    pub rssi: i8,
    pub latency_ms: u32,
    pub edv: Option<i8>, // Energy Detection Value for QoS
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
    Modulation(
        kaonic_radio::modulation::Modulation,
        oneshot::Sender<Result<(), String>>,
    ),
    Transmit(Kaonic1SFrame, oneshot::Sender<Result<u32, String>>),
    ConfigureQoS(QoSConfig, oneshot::Sender<Result<(), String>>),
    Shutdown,
}

#[derive(Clone, Debug)]
pub struct QoSConfig {
    pub enabled: bool,
    pub adaptive_modulation: bool,
    pub adaptive_tx_power: bool,
    pub adaptive_backoff: bool,
    pub cca_threshold: i8,
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

    pub async fn set_modulation(
        &self,
        module: usize,
        modulation: kaonic_radio::modulation::Modulation,
    ) -> Result<(), String> {
        let handle = self.get_worker(module)?;
        let (tx, rx) = oneshot::channel();
        handle
            .cmd_tx
            .send(RadioCommand::Modulation(modulation, tx))
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

    pub async fn configure_qos(&self, module: usize, config: QoSConfig) -> Result<(), String> {
        let handle = self.get_worker(module)?;
        let (tx, rx) = oneshot::channel();
        handle
            .cmd_tx
            .send(RadioCommand::ConfigureQoS(config, tx))
            .map_err(|_| "Worker thread stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("Worker dropped".into()))
    }

    pub fn subscribe(&self, module: usize) -> Result<broadcast::Receiver<ReceiveEvent>, String> {
        let handle = self.get_worker(module)?;
        Ok(handle.rx_tx.subscribe())
    }

    // Signal all workers to stop and close receive channels so gRPC streams end.
    pub fn shutdown(&self) {
        for w in &self.workers {
            if let Some(w) = w.as_ref() {
                let _ = w.cmd_tx.send(RadioCommand::Shutdown);
                // Subscribers will also observe external shutdown signal from gRPC layer.
            }
        }
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
    let mut qos_manager: Option<QoSManager> = None;
    let mut manual_modulation: Option<kaonic_radio::modulation::Modulation> = None;
    let mut last_qos_modulation: Option<kaonic_radio::modulation::Modulation> = None;
    let mut idle_edv_counter = 0u32;

    loop {
        // Drain any available commands before attempting receive
        match cmd_rx.try_recv() {
            Ok(RadioCommand::Configure(cfg, ack)) => {
                let res = radio
                    .configure(&cfg)
                    .map_err(|e| format!("configure failed: {:?}", e));
                let _ = ack.send(res);
            }
            Ok(RadioCommand::Modulation(modulation, ack)) => {
                // Save manual modulation
                manual_modulation = Some(modulation.clone());
                
                // Only apply it if QoS is disabled
                let res = if qos_manager.is_none() {
                    radio
                        .set_modulation(&modulation)
                        .map_err(|e| format!("set_modulation failed: {:?}", e))
                } else {
                    log::debug!("Module {}: Manual modulation saved but not applied (QoS enabled)", module);
                    Ok(())
                };
                let _ = ack.send(res);
            }
            Ok(RadioCommand::Transmit(frame, ack)) => {
                let start = Instant::now();
                let res = radio
                    .transmit(&frame)
                    .map(|_| start.elapsed().as_millis() as u32)
                    .map_err(|e| format!("transmit failed: {:?}", e));
                let _ = ack.send(res);
            }
            Ok(RadioCommand::ConfigureQoS(config, ack)) => {
                if config.enabled {
                    let mut qos = QoSManager::new()
                        .with_cca_threshold(config.cca_threshold)
                        .enable_adaptive_modulation(config.adaptive_modulation)
                        .enable_adaptive_tx_power(config.adaptive_tx_power)
                        .enable_adaptive_backoff(config.adaptive_backoff);

                    // Use manual modulation as base if available
                    if let Some(ref manual_mod) = manual_modulation {
                        let scheme = match manual_mod {
                            kaonic_radio::modulation::Modulation::Ofdm(ofdm) => {
                                ModulationScheme::Ofdm(*ofdm)
                            }
                            kaonic_radio::modulation::Modulation::Qpsk(qpsk) => {
                                ModulationScheme::Qpsk(*qpsk)
                            }
                        };
                        qos = qos.with_default_modulation(scheme);
                    } else {
                        // Use default OFDM if no manual modulation set
                        qos = qos.with_modulation_type(kaonic_qos::ModulationType::Ofdm);
                    }

                    // Apply initial QoS modulation
                    let recommended = qos.get_modulation();
                    if let Err(e) = radio.set_modulation(&recommended) {
                        log::warn!("Module {}: Failed to set QoS modulation: {:?}", module, e);
                    } else {
                        log::info!("Module {}: QoS enabled, modulation set to {:?}", module, recommended);
                    }
                    
                    qos_manager = Some(qos);
                    let _ = ack.send(Ok(()));
                } else {
                    qos_manager = None;
                    last_qos_modulation = None;
                    
                    // Restore manual modulation when QoS is disabled
                    if let Some(ref manual_mod) = manual_modulation {
                        if let Err(e) = radio.set_modulation(manual_mod) {
                            log::warn!("Module {}: Failed to restore manual modulation: {:?}", module, e);
                        } else {
                            log::info!("Module {}: QoS disabled, manual modulation restored", module);
                        }
                    } else {
                        log::info!("Module {}: QoS disabled", module);
                    }
                    
                    let _ = ack.send(Ok(()));
                }
            }
            Ok(RadioCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                break;
            }
            Err(TryRecvError::Empty) => {
                // No command, fall through to receive
            }
        }

        // Update QoS with idle EDV periodically (every 10 cycles ~200ms)
        if let Some(ref mut qos) = qos_manager {
            idle_edv_counter += 1;
            if idle_edv_counter >= 10 {
                idle_edv_counter = 0;
                if let Ok(edv) = read_edv(&mut radio) {
                    qos.update_idle_edv(edv);
                    
                    // Check if modulation recommendation changed
                    let recommended = qos.get_modulation();
                    if last_qos_modulation.as_ref() != Some(&recommended) {
                        if let Err(e) = radio.set_modulation(&recommended) {
                            log::warn!("Module {}: Failed to update QoS modulation: {:?}", module, e);
                        } else {
                            log::debug!("Module {}: QoS modulation updated to {:?}", module, recommended);
                            last_qos_modulation = Some(recommended);
                        }
                    }
                }
            }
        }

        let start = Instant::now();
        match radio.receive(&mut rx_frame, Duration::from_millis(20)) {
            Ok(rr) => {
                // Update QoS with RX EDV
                let edv = if qos_manager.is_some() {
                    let edv_result = read_edv(&mut radio);
                    if let Ok(edv) = edv_result {
                        if let Some(ref mut qos) = qos_manager {
                            qos.update_rx_edv(edv);
                            
                            // Check if modulation recommendation changed
                            let recommended = qos.get_modulation();
                            if last_qos_modulation.as_ref() != Some(&recommended) {
                                if let Err(e) = radio.set_modulation(&recommended) {
                                    log::warn!("Module {}: Failed to update QoS modulation: {:?}", module, e);
                                } else {
                                    log::debug!("Module {}: QoS modulation updated based on RX to {:?}", module, recommended);
                                    last_qos_modulation = Some(recommended);
                                }
                            }
                        }
                        Some(edv)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Copy data into a new frame so we don't move rx_frame out of scope
                let mut out_frame = Frame::<FRAME_SIZE>::new();
                out_frame.copy_from_slice(rx_frame.as_slice());
                let evt = ReceiveEvent {
                    module,
                    frame: out_frame,
                    rssi: rr.rssi,
                    latency_ms: start.elapsed().as_millis() as u32,
                    edv,
                };
                let _ = rx_tx.send(evt);
            }
            Err(_e) => {
                // Timeout or hw error during idle receive; ignore and continue
            }
        }
    }
}

// Helper function to read EDV from radio
fn read_edv(radio: &mut kaonic_radio::platform::kaonic1s::Kaonic1SRadio) -> Result<i8, String> {
    radio
        .radio()
        .read_edv()
        .map_err(|e| format!("Failed to read EDV: {:?}", e))
}

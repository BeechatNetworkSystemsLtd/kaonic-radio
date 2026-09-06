//! Radio link quality test: drives two kaonic-commd devices over the ctrl
//! protocol, sweeps modulations from most robust to fastest, and reports
//! loss / latency / throughput / RSSI per modulation.

use std::time::{Duration, Instant};

use clap::Parser;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use kaonic_ctrl::{
    client::Client,
    protocol::{MessageCoder, ReceiveModule},
    radio::RadioClient,
};
use kaonic_frame::frame::Frame;
use radio_common::{
    modulation::{OfdmBandwidthOption, OfdmMcs, OfdmModulation},
    Hertz, Modulation, RadioConfigBuilder,
};

const MAGIC: [u8; 4] = *b"LNKT";
const HEADER_SIZE: usize = 16; // MAGIC(4) + SEQ(4) + SEND_US(8)

#[derive(Parser, Debug)]
#[command(name = "kaonic-linktest", about = "Kaonic radio link quality sweep")]
struct Args {
    /// Transmitting device (kaonic-commd ctrl address)
    #[arg(long, default_value = "kaonic1s-6v724iaxjiyxba4w.local:9090")]
    tx: String,

    /// Receiving device (kaonic-commd ctrl address)
    #[arg(long, default_value = "kaonic1s-7qjt6enxiargc7sk.local:9090")]
    rx: String,

    /// Radio module index on the transmitting device
    #[arg(long, default_value_t = 0)]
    module: usize,

    /// Radio module index on the receiving device (defaults to --module)
    #[arg(long)]
    rx_module: Option<usize>,

    /// Force this channel on both sides before testing
    #[arg(long)]
    channel: Option<u16>,

    /// Center frequency in kHz when forcing config (with --channel)
    #[arg(long, default_value_t = 869_535)]
    freq_khz: u64,

    /// Frames per modulation
    #[arg(long, default_value_t = 50)]
    count: u32,

    /// Frame payload size in bytes
    #[arg(long, default_value_t = 256)]
    size: usize,

    /// Gap between frames in ms (0 = back-to-back)
    #[arg(long, default_value_t = 20)]
    interval_ms: u64,

    /// TX power (0..31)
    #[arg(long, default_value_t = 10)]
    tx_power: u8,

    /// Run only one MCS (0..=6); default sweeps all
    #[arg(long)]
    mcs: Option<u8>,

    /// Which side(s) to send set_modulation to: both | tx | rx | none
    #[arg(long, default_value = "both")]
    config_side: String,

    /// Number of parallel transmit connections (keeps the daemon's TX queue
    /// non-empty so the radio never idles between frames)
    #[arg(long, default_value_t = 1)]
    pipeline: usize,

    /// Use the batch transmit API with this many frames per batch (0 = off)
    #[arg(long, default_value_t = 0)]
    batch: usize,
}

fn mcs_list() -> Vec<(&'static str, OfdmMcs)> {
    vec![
        ("BPSK 1/2 4x", OfdmMcs::BpskC1_2_4x),
        ("BPSK 1/2 2x", OfdmMcs::BpskC1_2_2x),
        ("QPSK 1/2 2x", OfdmMcs::QpskC1_2_2x),
        ("QPSK 1/2", OfdmMcs::QpskC1_2),
        ("QPSK 3/4", OfdmMcs::QpskC3_4),
        ("16QAM 1/2", OfdmMcs::QamC1_2),
        ("16QAM 3/4", OfdmMcs::QamC3_4),
    ]
}

async fn connect(addr: &str, cancel: CancellationToken) -> Result<RadioClient, String> {
    let server_addr = tokio::net::lookup_host(addr)
        .await
        .map_err(|e| format!("resolve {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("no address for {}", addr))?;

    let client = Client::connect(
        "0.0.0.0:0".parse().unwrap(),
        server_addr,
        MessageCoder::<1400, 5>::new(),
        cancel.clone(),
    )
    .await
    .map_err(|e| format!("connect {}: {:?}", addr, e))?;

    RadioClient::new(client, cancel)
        .await
        .map_err(|e| format!("client {}: {:?}", addr, e))
}

struct RxStats {
    received: u32,
    latency_sum_us: u64,
    latency_max_us: u64,
    rssi_sum: i64,
    rssi_min: i8,
    rssi_max: i8,
}

struct Report {
    name: &'static str,
    sent: u32,
    rx: RxStats,
    elapsed: Duration,
    bytes: u64,
    tx_errors: u32,
}

impl Report {
    fn print(&self) {
        let loss =
            (self.sent.saturating_sub(self.rx.received)) as f64 * 100.0 / self.sent.max(1) as f64;
        let avg_lat = self.rx.latency_sum_us as f64 / self.rx.received.max(1) as f64 / 1000.0;
        let kbps = self.bytes as f64 * 8.0 / self.elapsed.as_secs_f64() / 1000.0;
        let rssi_avg = self.rx.rssi_sum as f64 / self.rx.received.max(1) as f64;

        println!(
            "{:<12} | sent {:>4} rx {:>4} loss {:>5.1}% txerr {:>2} | lat avg {:>7.2}ms max {:>7.2}ms | {:>8.1} kbit/s | rssi {:>4}/{:>5.1}/{:>4} dBm",
            self.name,
            self.sent,
            self.rx.received,
            loss,
            self.tx_errors,
            avg_lat,
            self.rx.latency_max_us as f64 / 1000.0,
            kbps,
            self.rx.rssi_min,
            rssi_avg,
            self.rx.rssi_max,
        );
    }
}

async fn run_one(
    args: &Args,
    tx: &mut RadioClient,
    rx: &mut RadioClient,
    name: &'static str,
    mcs: OfdmMcs,
) -> Result<Report, String> {
    let rx_module = args.rx_module.unwrap_or(args.module);

    if let Some(channel) = args.channel {
        let config = RadioConfigBuilder::new()
            .freq(Hertz::new(args.freq_khz * 1000))
            .channel(channel)
            .build();

        tx.set_radio_config(args.module, config)
            .await
            .map_err(|e| format!("tx set_radio_config: {:?}", e))?;
        rx.set_radio_config(rx_module, config)
            .await
            .map_err(|e| format!("rx set_radio_config: {:?}", e))?;
    }

    let modulation = Modulation::Ofdm(OfdmModulation {
        mcs,
        opt: OfdmBandwidthOption::Option1,
        tx_power: args.tx_power,
    });

    if args.config_side == "both" || args.config_side == "tx" {
        tx.set_modulation(args.module, modulation)
            .await
            .map_err(|e| format!("tx set_modulation: {:?}", e))?;
    }
    if args.config_side == "both" || args.config_side == "rx" {
        rx.set_modulation(rx_module, modulation)
            .await
            .map_err(|e| format!("rx set_modulation: {:?}", e))?;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut rx_events = rx.module_receive();

    // Drain anything stale
    while rx_events.try_recv().is_ok() {}

    let epoch = Instant::now();
    let count = args.count;

    let collector = tokio::spawn(async move {
        collect(rx_events, epoch, rx_module, count).await
    });

    let size = args.size.clamp(HEADER_SIZE, 2048);
    let mut tx_errors = 0u32;
    let mut sent = 0u32;
    let mut bytes = 0u64;

    let start = Instant::now();

    if args.batch > 0 {
        let mut batch_frames: Vec<Frame<2048>> = Vec::new();
        let mut seq = 0u32;

        while seq < args.count {
            batch_frames.clear();

            while batch_frames.len() < args.batch && seq < args.count {
                let mut frame = Frame::<2048>::new();
                let buf = frame.alloc_buffer(size).expect("frame size");
                buf[..4].copy_from_slice(&MAGIC);
                buf[4..8].copy_from_slice(&seq.to_le_bytes());
                buf[8..16]
                    .copy_from_slice(&(epoch.elapsed().as_micros() as u64).to_le_bytes());
                for (i, b) in buf[HEADER_SIZE..].iter_mut().enumerate() {
                    *b = (i & 0xFF) as u8;
                }
                batch_frames.push(frame);
                seq += 1;
            }

            match tx.transmit_batch(args.module, &batch_frames).await {
                Ok((ok, err)) => {
                    sent += ok;
                    tx_errors += err;
                    bytes += u64::from(ok) * size as u64;
                }
                Err(e) => {
                    log::debug!("batch error: {:?}", e);
                    tx_errors += batch_frames.len() as u32;
                }
            }
        }
    } else if args.pipeline > 1 {
        // Interleave frames over N connections; while one frame is on air the
        // daemon already holds the next request in its queue.
        let mut workers = Vec::new();

        for lane in 0..args.pipeline {
            let addr = args.tx.clone();
            let module = args.module;
            let count = args.count;
            let lanes = args.pipeline;
            let cancel = CancellationToken::new();

            workers.push(tokio::spawn(async move {
                let mut client = match connect(&addr, cancel).await {
                    Ok(c) => c,
                    Err(_) => return (0u32, 0u32, 0u64),
                };

                let mut frame = Frame::<2048>::new();
                let (mut sent, mut errors, mut bytes) = (0u32, 0u32, 0u64);
                let mut seq = lane as u32;

                while seq < count {
                    frame.clear();
                    let buf = frame.alloc_buffer(size).expect("frame size");
                    buf[..4].copy_from_slice(&MAGIC);
                    buf[4..8].copy_from_slice(&seq.to_le_bytes());
                    buf[8..16]
                        .copy_from_slice(&(epoch.elapsed().as_micros() as u64).to_le_bytes());
                    for (i, b) in buf[HEADER_SIZE..].iter_mut().enumerate() {
                        *b = (i & 0xFF) as u8;
                    }

                    match client.transmit(module, &frame).await {
                        Ok(_) => {
                            sent += 1;
                            bytes += size as u64;
                        }
                        Err(_) => errors += 1,
                    }

                    seq += lanes as u32;
                }

                (sent, errors, bytes)
            }));
        }

        for worker in workers {
            if let Ok((s, e, b)) = worker.await {
                sent += s;
                tx_errors += e;
                bytes += b;
            }
        }
    } else {
        let mut frame = Frame::<2048>::new();

        for seq in 0..args.count {
            frame.clear();
            let buf = frame.alloc_buffer(size).expect("frame size");
            buf[..4].copy_from_slice(&MAGIC);
            buf[4..8].copy_from_slice(&seq.to_le_bytes());
            buf[8..16].copy_from_slice(&(epoch.elapsed().as_micros() as u64).to_le_bytes());
            for (i, b) in buf[HEADER_SIZE..].iter_mut().enumerate() {
                *b = (i & 0xFF) as u8;
            }

            match tx.transmit(args.module, &frame).await {
                Ok(_) => {
                    sent += 1;
                    bytes += size as u64;
                }
                Err(e) => {
                    log::debug!("tx error seq {}: {:?}", seq, e);
                    tx_errors += 1;
                }
            }

            if args.interval_ms > 0 {
                tokio::time::sleep(Duration::from_millis(args.interval_ms)).await;
            }
        }
    }

    let elapsed = start.elapsed();

    // Grace period for stragglers
    let rx_stats = match tokio::time::timeout(Duration::from_secs(2), collector).await {
        Ok(Ok(stats)) => stats,
        _ => RxStats {
            received: 0,
            latency_sum_us: 0,
            latency_max_us: 0,
            rssi_sum: 0,
            rssi_min: 0,
            rssi_max: 0,
        },
    };

    Ok(Report {
        name,
        sent,
        rx: rx_stats,
        elapsed,
        bytes,
        tx_errors,
    })
}

async fn collect(
    mut rx_events: broadcast::Receiver<Box<ReceiveModule>>,
    epoch: Instant,
    module: usize,
    expected: u32,
) -> RxStats {
    let mut stats = RxStats {
        received: 0,
        latency_sum_us: 0,
        latency_max_us: 0,
        rssi_sum: 0,
        rssi_min: i8::MAX,
        rssi_max: i8::MIN,
    };
    let mut seen = vec![false; expected as usize];

    loop {
        // Stop once everything arrived, or after 1.5s of silence
        let event = match tokio::time::timeout(Duration::from_millis(1500), rx_events.recv()).await
        {
            Ok(Ok(ev)) => ev,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            _ => break,
        };

        if event.module != module {
            continue;
        }

        let data = event.frame.as_slice();
        if data.len() < HEADER_SIZE || data[..4] != MAGIC {
            continue;
        }

        let seq = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let send_us = u64::from_le_bytes(data[8..16].try_into().unwrap());

        if seq >= seen.len() || seen[seq] {
            continue;
        }
        seen[seq] = true;

        let latency_us = (epoch.elapsed().as_micros() as u64).saturating_sub(send_us);

        stats.received += 1;
        stats.latency_sum_us += latency_us;
        stats.latency_max_us = stats.latency_max_us.max(latency_us);
        stats.rssi_sum += i64::from(event.rssi);
        stats.rssi_min = stats.rssi_min.min(event.rssi);
        stats.rssi_max = stats.rssi_max.max(event.rssi);

        if stats.received == expected {
            break;
        }
    }

    if stats.received == 0 {
        stats.rssi_min = 0;
        stats.rssi_max = 0;
    }

    stats
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let args = Args::parse();
    let cancel = CancellationToken::new();

    println!("linktest: tx={} rx={} module={}", args.tx, args.rx, args.module);
    println!(
        "frames={} size={}B interval={}ms tx_power={}",
        args.count, args.size, args.interval_ms, args.tx_power
    );

    let mut tx = connect(&args.tx, cancel.clone()).await?;
    let mut rx = connect(&args.rx, cancel.clone()).await?;

    tx.ping().await.map_err(|e| format!("tx ping: {:?}", e))?;
    rx.ping().await.map_err(|e| format!("rx ping: {:?}", e))?;

    let info_tx = tx.get_info().await.map_err(|e| format!("{:?}", e))?;
    let info_rx = rx.get_info().await.map_err(|e| format!("{:?}", e))?;
    println!(
        "tx: {} v{} | rx: {} v{}",
        info_tx.serial, info_tx.version, info_rx.serial, info_rx.version
    );
    println!();

    let list = mcs_list();
    let selected: Vec<_> = match args.mcs {
        Some(i) => vec![list[(i as usize).min(list.len() - 1)]],
        None => list,
    };

    let mut reports = Vec::new();

    for (name, mcs) in selected {
        eprintln!("running {} ...", name);
        match run_one(&args, &mut tx, &mut rx, name, mcs).await {
            Ok(report) => {
                report.print();
                reports.push(report);
            }
            Err(e) => println!("{:<12} | FAILED: {}", name, e),
        }
    }

    println!();
    println!("summary ({} frames x {}B per modulation):", args.count, args.size);
    for r in &reports {
        r.print();
    }

    cancel.cancel();
    Ok(())
}

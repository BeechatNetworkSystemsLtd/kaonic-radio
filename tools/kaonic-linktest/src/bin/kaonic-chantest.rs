//! Channel test: opens the same channel on two daemons, sends payloads through
//! one and counts what the other delivers. Exercises the payload-level path —
//! framing, coding, bundling and delivery all happen in the daemons.
//!
//! Run it on the receiving node so its daemon is reached over loopback:
//!
//! ```text
//! kaonic-chantest --tx <other-node>:9090 --rx 127.0.0.1:9090 --profile voice --count 500 --size 60
//! ```

use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use tokio_util::sync::CancellationToken;

use kaonic_ctrl::channel::{ChannelId, ProfileSpec};
use kaonic_ctrl::{client::Client, protocol::MessageCoder, radio::RadioClient};
use kaonic_fec::{FecCode, TrafficClass};

#[derive(Parser)]
#[command(name = "kaonic-chantest", about = "Kaonic channel round-trip test")]
struct Args {
    /// Daemon that transmits.
    #[arg(long, default_value = "kaonic1s-6v724iaxjiyxba4w.local:9090")]
    tx: String,
    /// Daemon that receives; run this tool on that node and use loopback.
    #[arg(long, default_value = "127.0.0.1:9090")]
    rx: String,
    /// Radio module on both daemons.
    #[arg(long, default_value_t = 0)]
    module: usize,
    /// Channel name; both ends derive the same id from it.
    #[arg(long, default_value = "kaonic-chantest")]
    channel: String,
    #[arg(long, value_enum, default_value_t = ProfileArg::Robust)]
    profile: ProfileArg,
    #[arg(long, default_value_t = 200)]
    count: u32,
    /// Payload size in bytes.
    #[arg(long, default_value_t = 800)]
    size: usize,
    /// Gap between payloads in milliseconds (0 = as fast as the daemon accepts).
    #[arg(long, default_value_t = 0)]
    interval_ms: u64,
    /// Concurrent senders; each `send` is a round trip to the transmitting
    /// daemon, so several in flight keep its queue from running dry.
    #[arg(long, default_value_t = 4)]
    pipeline: u32,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileArg {
    Robust,
    Bulk,
    Voice,
    Raw,
}

impl ProfileArg {
    fn spec(self, interval_ms: u64) -> ProfileSpec {
        match self {
            ProfileArg::Robust => ProfileSpec::Robust,
            ProfileArg::Bulk => ProfileSpec::Bulk {
                class: TrafficClass::Auto,
            },
            ProfileArg::Voice => ProfileSpec::Voice {
                code: FecCode::Tm1280,
                packet_interval: Duration::from_millis(interval_ms.max(20)),
            },
            ProfileArg::Raw => ProfileSpec::Raw,
        }
    }
}

async fn connect(addr: &str, cancel: CancellationToken) -> Result<RadioClient, String> {
    let server_addr = tokio::net::lookup_host(addr)
        .await
        .map_err(|e| format!("resolve {addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("no address for {addr}"))?;
    let client = Client::connect(
        "0.0.0.0:0".parse().unwrap(),
        server_addr,
        MessageCoder::<1400, 5>::new(),
        cancel.clone(),
    )
    .await
    .map_err(|e| format!("connect {addr}: {e:?}"))?;
    RadioClient::new(client, cancel)
        .await
        .map_err(|e| format!("client {addr}: {e:?}"))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();
    let args = Args::parse();
    let cancel = CancellationToken::new();

    let mut tx_radio = connect(&args.tx, cancel.clone()).await?;
    let mut rx_radio = connect(&args.rx, cancel.clone()).await?;
    tx_radio.ping().await.map_err(|e| format!("tx ping: {e:?}"))?;
    rx_radio.ping().await.map_err(|e| format!("rx ping: {e:?}"))?;

    let id = ChannelId::of(&args.channel);
    let spec = args.profile.spec(args.interval_ms);
    println!(
        "channel {:?} id={} module={} profile={:?}",
        args.channel,
        id.raw(),
        args.module,
        spec
    );

    // Receiver first, so nothing is on the air before someone listens.
    let mut rx = rx_radio
        .channel(id)
        .modules([args.module])
        .profile(spec.clone())
        .build_rx()
        .await
        .map_err(|e| format!("open rx: {e}"))?;
    let tx = tx_radio
        .channel(id)
        .modules([args.module])
        .profile(spec)
        .build_tx()
        .await
        .map_err(|e| format!("open tx: {e}"))?;
    println!(
        "opened: mtu {} B, frame capacity {} B",
        tx.mtu(),
        tx.frame_capacity()
    );

    let size = args.size.clamp(12, tx.mtu());
    let count = args.count;
    let epoch = Instant::now();

    let rx_stats_handle = rx.resubscribe();
    let collector = tokio::spawn(async move {
        let mut seen = vec![false; count as usize];
        let (mut received, mut dup, mut lat_sum, mut lat_max, mut rssi_sum) = (0u32, 0u32, 0u128, 0u128, 0i64);
        loop {
            match tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await {
                Ok(Ok(pkt)) => {
                    if pkt.payload.len() < 12 {
                        continue;
                    }
                    let seq = u32::from_le_bytes(pkt.payload[..4].try_into().unwrap()) as usize;
                    let sent_us = u64::from_le_bytes(pkt.payload[4..12].try_into().unwrap());
                    if seq >= seen.len() {
                        continue;
                    }
                    if seen[seq] {
                        dup += 1;
                        continue;
                    }
                    seen[seq] = true;
                    received += 1;
                    let lat = (epoch.elapsed().as_micros()).saturating_sub(u128::from(sent_us));
                    lat_sum += lat;
                    lat_max = lat_max.max(lat);
                    rssi_sum += i64::from(pkt.info.rssi);
                    if received == count {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        (received, dup, lat_sum, lat_max, rssi_sum)
    });

    let start = Instant::now();
    let pipeline = args.pipeline.max(1);
    let interval = args.interval_ms;
    let mut workers = Vec::new();
    for lane in 0..pipeline {
        let tx = tx.clone();
        workers.push(tokio::spawn(async move {
            let mut payload = vec![0u8; size];
            for (i, b) in payload.iter_mut().enumerate().skip(12) {
                *b = (i & 0xFF) as u8;
            }
            let (mut sent, mut failed) = (0u32, 0u32);
            let mut seq = lane;
            while seq < count {
                payload[..4].copy_from_slice(&seq.to_le_bytes());
                payload[4..12].copy_from_slice(&(epoch.elapsed().as_micros() as u64).to_le_bytes());
                match tx.send(&payload).await {
                    Ok(()) => sent += 1,
                    Err(e) => {
                        failed += 1;
                        log::debug!("send {seq}: {e}");
                    }
                }
                if interval > 0 {
                    tokio::time::sleep(Duration::from_millis(interval)).await;
                }
                seq += pipeline;
            }
            (sent, failed)
        }));
    }
    let (mut sent, mut failed) = (0u32, 0u32);
    for worker in workers {
        let (s, f) = worker.await.unwrap();
        sent += s;
        failed += f;
    }
    let elapsed = start.elapsed();

    let (received, dup, lat_sum, lat_max, rssi_sum) = collector.await.unwrap();
    let loss = (sent.saturating_sub(received)) as f64 * 100.0 / sent.max(1) as f64;
    let kbps = (sent as f64) * (size as f64) * 8.0 / elapsed.as_secs_f64() / 1000.0;
    let goodput = (received as f64) * (size as f64) / elapsed.as_secs_f64() / 1024.0;
    println!(
        "sent {sent} (failed {failed}) received {received} dup {dup} loss {loss:.1}% | {kbps:.0} kbit/s offered, {goodput:.1} KB/s delivered | lat avg {:.1} ms max {:.1} ms | rssi {:.1} dBm | {:.1}s",
        lat_sum as f64 / received.max(1) as f64 / 1000.0,
        lat_max as f64 / 1000.0,
        rssi_sum as f64 / received.max(1) as f64,
        elapsed.as_secs_f64(),
    );

    if let Ok(stats) = tx.stats().await {
        println!("tx daemon: frames {} errors {} dropped {}", stats.tx_frames, stats.tx_errors, stats.tx_dropped);
    }
    if let Ok(stats) = rx_stats_handle.stats().await {
        println!(
            "rx daemon: frames {} payloads {} decode clean {} corrected {} failed {}",
            stats.rx_frames, stats.rx_payloads, stats.decode_clean, stats.decode_corrected, stats.decode_failed
        );
    }
    cancel.cancel();
    Ok(())
}

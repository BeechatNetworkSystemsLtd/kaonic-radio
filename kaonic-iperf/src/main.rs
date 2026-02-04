use clap::Parser;
use crc32fast::Hasher;
use log::{error, warn};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

mod config;

use kaonic::{radio_client::RadioClient, RadioFrame, ReceiveRequest, TransmitRequest};

const DEFAULT_COMMD_ADDR: &str = "http://192.168.10.1:8080";
const MIN_PACKET_SIZE: usize = 24; // MAGIC(4) + SEQ(4) + TIMESTAMP(8) + padding(4) + CRC(4)
const MAX_PACKET_SIZE: usize = 2048;
const MAX_WORDS: usize = MAX_PACKET_SIZE / 4;

/// 4-byte aligned buffer for zero-copy u8 <-> u32 conversion
#[repr(C, align(4))]
struct AlignedBuffer {
    data: [u8; MAX_PACKET_SIZE],
    len: usize,
}

impl AlignedBuffer {
    fn new() -> Self {
        Self {
            data: [0u8; MAX_PACKET_SIZE],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    fn as_words(&self) -> &[u32] {
        let word_len = (self.len + 3) / 4;
        // SAFETY: buffer is aligned to 4 bytes, and we're on little-endian
        unsafe { std::slice::from_raw_parts(self.data.as_ptr() as *const u32, word_len) }
    }

    fn set_len(&mut self, len: usize) {
        self.len = len.min(MAX_PACKET_SIZE);
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, byte: u8) {
        if self.len < MAX_PACKET_SIZE {
            self.data[self.len] = byte;
            self.len += 1;
        }
    }

    fn extend_from_slice(&mut self, slice: &[u8]) {
        let copy_len = slice.len().min(MAX_PACKET_SIZE - self.len);
        self.data[self.len..self.len + copy_len].copy_from_slice(&slice[..copy_len]);
        self.len += copy_len;
    }

    fn len(&self) -> usize {
        self.len
    }
}
const RESPONSE_TIMEOUT_MS: u64 = 500;

#[derive(Parser, Debug)]
#[command(name = "kaonic-iperf")]
#[command(about = "Simple RTT and throughput measurement for Kaonic radio")]
struct Args {
    /// Path to kaonic-config.toml
    #[arg(long, short = 'c', default_value = "kaonic-config.toml")]
    config: String,

    /// kaonic-commd gRPC address (overrides config file)
    #[arg(long, short = 'a')]
    address: Option<String>,

    /// Run as server (responder)
    #[arg(long, conflicts_with = "client")]
    server: bool,

    /// Run as client (initiator)
    #[arg(long, conflicts_with = "server")]
    client: bool,
}

// Packet structure:
// MAGIC (4) + SEQ (4) + TIMESTAMP (8) + PADDING (N) + CRC32 (4)
// Minimum size: 24 bytes
const MAGIC: [u8; 4] = [0x8B, 0x52, 0x54, 0x54];

fn compute_crc(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

fn encode_frame(buffer: &AlignedBuffer) -> RadioFrame {
    RadioFrame {
        data: buffer.as_words().to_vec(),
        length: buffer.len() as u32,
    }
}

fn decode_frame_into(frame: &RadioFrame, buffer: &mut AlignedBuffer) {
    let byte_len = frame.length as usize;
    let word_len = frame.data.len().min(MAX_WORDS);

    // SAFETY: we're copying u32 words directly into aligned buffer as bytes (little-endian)
    unsafe {
        let src = frame.data.as_ptr() as *const u8;
        let dst = buffer.data.as_mut_ptr();
        std::ptr::copy_nonoverlapping(src, dst, word_len * 4);
    }

    buffer.set_len(byte_len);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn fill_packet(packet: &mut AlignedBuffer, seq: u32, size: usize) {
    let size = size.clamp(MIN_PACKET_SIZE, MAX_PACKET_SIZE);
    packet.clear();

    // Header: MAGIC + SEQ + TIMESTAMP (16 bytes)
    packet.extend_from_slice(&MAGIC);
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&now_ms().to_le_bytes());

    // Padding (fill to size - 4 bytes for CRC)
    while packet.len() < size - 4 {
        packet.push((packet.len() & 0xFF) as u8);
    }

    // CRC32 of everything before it
    let crc = compute_crc(packet.as_bytes());
    packet.extend_from_slice(&crc.to_le_bytes());
}

#[derive(Debug)]
enum ParseError {
    TooShort,
    BadMagic,
    CrcMismatch { expected: u32, actual: u32 },
}

/// Returns (seq, timestamp) if packet is valid
fn parse_packet(data: &[u8]) -> Result<(u32, u64), ParseError> {
    if data.len() < MIN_PACKET_SIZE {
        return Err(ParseError::TooShort);
    }

    // Check magic
    if data[0..4] != MAGIC {
        return Err(ParseError::BadMagic);
    }

    // Verify CRC (last 4 bytes)
    let payload_end = data.len() - 4;
    let expected_crc = u32::from_le_bytes([
        data[payload_end],
        data[payload_end + 1],
        data[payload_end + 2],
        data[payload_end + 3],
    ]);
    let actual_crc = compute_crc(&data[..payload_end]);

    if expected_crc != actual_crc {
        return Err(ParseError::CrcMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }

    // Parse header
    let seq = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let timestamp = u64::from_le_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);

    Ok((seq, timestamp))
}

async fn run_server(address: &str, cfg: &config::Config) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Kaonic RTT Server ===");
    println!("Connecting to {}...", address);

    let mut client = RadioClient::connect(address.to_string()).await?;
    println!("Connected.");
    
    // Apply radio configuration for the target module only
    if let Some(radio_cfg) = cfg.radios.iter().find(|r| r.module == cfg.iperf.module) {
        println!("Configuring radio module {}...", cfg.iperf.module);
        client.configure(radio_cfg.clone()).await?;
        println!("Radio configuration applied.\n");
    } else {
        println!("Warning: no radio config found for module {}\n", cfg.iperf.module);
    }

    let request = ReceiveRequest {
        module: cfg.iperf.module,
        timeout: 0,
    };

    let mut stream = client.receive_stream(request).await?.into_inner();
    let mut count: u64 = 0;
    let mut ignored: u64 = 0;
    let mut crc_errors: u64 = 0;
    let mut bytes_received: u64 = 0;
    let mut start_time: Option<Instant> = None;

    // Pre-allocate reusable aligned buffer
    let mut rx_buf = AlignedBuffer::new();

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                println!("\nShutting down...");
                break;
            }
            result = stream.next() => {
                match result {
                    Some(Ok(response)) => {
                        let frame = response.frame.unwrap_or_default();
                        decode_frame_into(&frame, &mut rx_buf);

                        match parse_packet(rx_buf.as_bytes()) {
                            Ok((seq, _ts)) => {
                                // Track receive stats
                                let packet_size = rx_buf.len() as u64;
                                if start_time.is_none() {
                                    start_time = Some(Instant::now());
                                }
                                bytes_received += packet_size;

                                // Calculate current receive speed
                                let speed_kbps = start_time
                                    .map(|t| {
                                        let elapsed = t.elapsed().as_secs_f64();
                                        if elapsed > 0.0 {
                                            (bytes_received as f64 * 8.0) / elapsed / 1000.0
                                        } else {
                                            0.0
                                        }
                                    })
                                    .unwrap_or(0.0);

                                // Echo back the same packet (preserving timestamp and CRC)
                                let tx_request = TransmitRequest {
                                    module: cfg.iperf.module,
                                    frame: Some(frame),
                                };

                                match client.transmit(tx_request).await {
                                    Ok(_) => {
                                        count += 1;
                                        println!(
                                            "[{}] Echo seq={} size={} rssi={} dBm  rx={:.2} kb/s",
                                            count, seq, rx_buf.len(), response.rssi, speed_kbps
                                        );
                                    }
                                    Err(e) => warn!("Transmit error: {}", e),
                                }
                            }
                            Err(ParseError::TooShort) => {
                                ignored += 1;
                            }
                            Err(ParseError::BadMagic) => {
                                ignored += 1;
                            }
                            Err(ParseError::CrcMismatch { expected, actual }) => {
                                crc_errors += 1;
                                warn!(
                                    "CRC mismatch: expected={:#010x} actual={:#010x} size={}",
                                    expected, actual, rx_buf.len()
                                );
                            }
                        }
                    }
                    Some(Err(e)) => error!("Receive error: {}", e),
                    None => break,
                }
            }
        }
    }

    println!("\nTotal packets echoed: {}", count);
    if ignored > 0 {
        println!("Ignored (non-iperf): {}", ignored);
    }
    if crc_errors > 0 {
        println!("CRC errors: {}", crc_errors);
    }
    if let Some(start) = start_time {
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let avg_speed_kbps = (bytes_received as f64 * 8.0) / elapsed / 1000.0;
            println!("Bytes received: {}", bytes_received);
            println!("Avg receive speed: {:.2} kb/s", avg_speed_kbps);
        }
    }
    Ok(())
}

async fn run_client(
    address: &str,
    cfg: &config::Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let packet_size = cfg.iperf.payload_size.clamp(MIN_PACKET_SIZE, MAX_PACKET_SIZE);

    println!("=== Kaonic RTT Client ===");
    println!("Connecting to {}...", address);

    let mut client = RadioClient::connect(address.to_string()).await?;
    println!("Connected.");
    
    // Apply radio configuration for the target module only
    if let Some(radio_cfg) = cfg.radios.iter().find(|r| r.module == cfg.iperf.module) {
        println!("Configuring radio module {}...", cfg.iperf.module);
        client.configure(radio_cfg.clone()).await?;
        println!("Radio configuration applied.");
    } else {
        println!("Warning: no radio config found for module {}", cfg.iperf.module);
    }
    
    println!("Packet size: {} bytes", packet_size);
    println!("Duration: {} seconds\n", cfg.iperf.duration);

    // Start receive stream
    let request = ReceiveRequest {
        module: cfg.iperf.module,
        timeout: RESPONSE_TIMEOUT_MS as u32,
    };
    let mut stream = client.receive_stream(request).await?.into_inner();

    let start = Instant::now();
    let test_duration = Duration::from_secs(cfg.iperf.duration);
    let mut seq: u32 = 0;
    let mut rtt_min: u64 = u64::MAX;
    let mut rtt_max: u64 = 0;
    let mut rtt_sum: u64 = 0;
    let mut rtt_count: u64 = 0;
    let mut bytes_transferred: u64 = 0;
    let mut timeouts: u64 = 0;
    let mut crc_errors: u64 = 0;

    // Pre-allocate reusable aligned buffers
    let mut packet_buf = AlignedBuffer::new();
    let mut rx_buf = AlignedBuffer::new();

    while start.elapsed() < test_duration {
        // Send request packet
        fill_packet(&mut packet_buf, seq, packet_size);
        let send_time = Instant::now();

        let tx_request = TransmitRequest {
            module: cfg.iperf.module,
            frame: Some(encode_frame(&packet_buf)),
        };

        if let Err(e) = client.transmit(tx_request).await {
            error!("Transmit error: {}", e);
            seq = seq.wrapping_add(1);
            continue;
        }

        // Wait for response
        match timeout(Duration::from_millis(RESPONSE_TIMEOUT_MS), stream.next()).await {
            Ok(Some(Ok(response))) => {
                let rtt = send_time.elapsed().as_millis() as u64;
                decode_frame_into(&response.frame.unwrap_or_default(), &mut rx_buf);

                match parse_packet(rx_buf.as_bytes()) {
                    Ok((resp_seq, _)) => {
                        if resp_seq == seq {
                            rtt_min = rtt_min.min(rtt);
                            rtt_max = rtt_max.max(rtt);
                            rtt_sum += rtt;
                            rtt_count += 1;
                            bytes_transferred += (packet_size * 2) as u64; // req + resp

                            println!(
                                "seq={:<6} rtt={:<4} ms  rssi={:<4} dBm  size={}",
                                seq,
                                rtt,
                                response.rssi,
                                rx_buf.len()
                            );
                        }
                    }
                    Err(ParseError::CrcMismatch { expected, actual }) => {
                        crc_errors += 1;
                        println!(
                            "seq={:<6} CRC ERROR (expected={:#010x} actual={:#010x})",
                            seq, expected, actual
                        );
                    }
                    Err(_) => {
                        // Ignore non-iperf packets
                    }
                }
            }
            Ok(Some(Err(e))) => {
                warn!("Receive error: {}", e);
                timeouts += 1;
            }
            Ok(None) => {
                warn!("Stream ended");
                break;
            }
            Err(_) => {
                println!("seq={:<6} TIMEOUT", seq);
                timeouts += 1;
            }
        }

        seq = seq.wrapping_add(1);
    }

    // Print results
    let elapsed = start.elapsed().as_secs_f64();
    let packets_sent = seq as u64;

    println!("\n=== Results ===");
    println!("Duration:     {:.2} s", elapsed);
    println!("Packet size:  {} bytes", packet_size);
    println!(
        "Packets:      {} sent, {} received, {} timeouts, {} CRC errors",
        packets_sent, rtt_count, timeouts, crc_errors
    );

    if rtt_count > 0 {
        let avg_rtt = rtt_sum as f64 / rtt_count as f64;

        println!(
            "RTT:          min={} ms, avg={:.1} ms, max={} ms",
            rtt_min, avg_rtt, rtt_max
        );
    }

    if elapsed > 0.0 {
        let speed_kbps = (bytes_transferred as f64 * 8.0) / elapsed / 1000.0;
        println!("Speed:        {:.2} kb/s", speed_kbps);
    }

    if packets_sent > 0 {
        let loss = ((packets_sent - rtt_count) as f64 / packets_sent as f64) * 100.0;
        println!("Packet loss:  {:.1}%", loss);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_level(log::Level::Info)?;

    let args = Args::parse();

    // Load config from specified path (required)
    let cfg = match config::load_config(&args.config) {
        Ok(c) => {
            println!("Loaded config from {} with {} radio(s)", args.config, c.radios.len());
            c
        }
        Err(e) => {
            eprintln!("Error: could not load config file '{}': {}", args.config, e);
            std::process::exit(1);
        }
    };

    if !args.server && !args.client {
        eprintln!("Error: specify --server or --client");
        std::process::exit(1);
    }

    // Determine address: CLI arg takes precedence over config
    let address = if let Some(addr) = &args.address {
        // Wrap with http:// and :8080 if not already formatted
        if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.clone()
        } else {
            format!("http://{}:8080", addr)
        }
    } else if let Some(ip) = &cfg.iperf.ip {
        format!("http://{}:8080", ip)
    } else {
        DEFAULT_COMMD_ADDR.to_string()
    };

    if args.server {
        run_server(&address, &cfg).await?;
    } else {
        run_client(&address, &cfg).await?;
    }

    Ok(())
}

use clap::Parser;
use crc32fast::Hasher;
use log::{error, warn};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

use kaonic::{radio_client::RadioClient, RadioFrame, RadioModule, ReceiveRequest, TransmitRequest};

const DEFAULT_COMMD_ADDR: &str = "http://127.0.0.1:8080";
const DEFAULT_DURATION_SECS: u64 = 10;
const DEFAULT_PACKET_SIZE: usize = 256;
const MIN_PACKET_SIZE: usize = 24; // MAGIC(4) + SEQ(4) + TIMESTAMP(8) + padding(4) + CRC(4)
const MAX_PACKET_SIZE: usize = 2048;
const RESPONSE_TIMEOUT_MS: u64 = 2000;

#[derive(Parser, Debug)]
#[command(name = "kaonic-iperf")]
#[command(about = "Simple RTT and throughput measurement for Kaonic radio")]
struct Args {
    /// kaonic-commd gRPC address
    #[arg(default_value = DEFAULT_COMMD_ADDR)]
    address: String,

    /// Run as server (responder)
    #[arg(long, conflicts_with = "client")]
    server: bool,

    /// Run as client (initiator)
    #[arg(long, conflicts_with = "server")]
    client: bool,

    /// Test duration in seconds
    #[arg(long, default_value_t = DEFAULT_DURATION_SECS)]
    duration: u64,

    /// Packet payload size in bytes (24-2048)
    #[arg(long, short = 's', default_value_t = DEFAULT_PACKET_SIZE)]
    size: usize,
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

fn encode_frame(buffer: &[u8]) -> RadioFrame {
    let words = buffer
        .chunks(4)
        .map(|chunk| {
            let mut word = 0u32;
            for (i, &byte) in chunk.iter().enumerate() {
                word |= (byte as u32) << (i * 8);
            }
            word
        })
        .collect::<Vec<_>>();

    RadioFrame {
        data: words,
        length: buffer.len() as u32,
    }
}

fn decode_frame(frame: &RadioFrame) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(frame.length as usize);
    for word in &frame.data {
        for i in 0..4 {
            buffer.push(((word >> (i * 8)) & 0xFF) as u8);
        }
    }
    buffer.truncate(frame.length as usize);
    buffer
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn create_packet(seq: u32, size: usize) -> Vec<u8> {
    let size = size.clamp(MIN_PACKET_SIZE, MAX_PACKET_SIZE);
    let mut packet = Vec::with_capacity(size);

    // Header: MAGIC + SEQ + TIMESTAMP (16 bytes)
    packet.extend_from_slice(&MAGIC);
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&now_ms().to_le_bytes());

    // Padding (fill to size - 4 bytes for CRC)
    while packet.len() < size - 4 {
        packet.push((packet.len() & 0xFF) as u8);
    }

    // CRC32 of everything before it
    let crc = compute_crc(&packet);
    packet.extend_from_slice(&crc.to_le_bytes());

    packet
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

async fn run_server(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Kaonic RTT Server ===");
    println!("Connecting to {}...", address);

    let mut client = RadioClient::connect(address.to_string()).await?;
    println!("Connected. Waiting for packets...\n");

    let request = ReceiveRequest {
        module: RadioModule::ModuleA as i32,
        timeout: 0,
    };

    let mut stream = client.receive_stream(request).await?.into_inner();
    let mut count: u64 = 0;
    let mut ignored: u64 = 0;
    let mut crc_errors: u64 = 0;
    let mut bytes_received: u64 = 0;
    let mut start_time: Option<Instant> = None;

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
                        let data = decode_frame(&frame);

                        match parse_packet(&data) {
                            Ok((seq, _ts)) => {
                                // Track receive stats
                                let packet_size = data.len() as u64;
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
                                    module: RadioModule::ModuleA as i32,
                                    frame: Some(frame),
                                };

                                match client.transmit(tx_request).await {
                                    Ok(_) => {
                                        count += 1;
                                        println!(
                                            "[{}] Echo seq={} size={} rssi={} dBm  rx={:.2} kb/s",
                                            count, seq, data.len(), response.rssi, speed_kbps
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
                                    expected, actual, data.len()
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
    duration_secs: u64,
    packet_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let packet_size = packet_size.clamp(MIN_PACKET_SIZE, MAX_PACKET_SIZE);

    println!("=== Kaonic RTT Client ===");
    println!("Connecting to {}...", address);

    let mut client = RadioClient::connect(address.to_string()).await?;
    println!("Connected.");
    println!("Packet size: {} bytes", packet_size);
    println!("Duration: {} seconds\n", duration_secs);

    // Start receive stream
    let request = ReceiveRequest {
        module: RadioModule::ModuleA as i32,
        timeout: RESPONSE_TIMEOUT_MS as u32,
    };
    let mut stream = client.receive_stream(request).await?.into_inner();

    let start = Instant::now();
    let test_duration = Duration::from_secs(duration_secs);
    let mut seq: u32 = 0;
    let mut rtts: Vec<u64> = Vec::new();
    let mut bytes_transferred: u64 = 0;
    let mut timeouts: u64 = 0;
    let mut crc_errors: u64 = 0;

    while start.elapsed() < test_duration {
        // Send request packet
        let packet = create_packet(seq, packet_size);
        let send_time = Instant::now();

        let tx_request = TransmitRequest {
            module: RadioModule::ModuleA as i32,
            frame: Some(encode_frame(&packet)),
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
                let frame_data = decode_frame(&response.frame.unwrap_or_default());

                match parse_packet(&frame_data) {
                    Ok((resp_seq, _)) => {
                        if resp_seq == seq {
                            rtts.push(rtt);
                            bytes_transferred += (packet_size * 2) as u64; // req + resp

                            println!(
                                "seq={:<6} rtt={:<4} ms  rssi={:<4} dBm  size={}",
                                seq,
                                rtt,
                                response.rssi,
                                frame_data.len()
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
    let packets_received = rtts.len() as u64;

    println!("\n=== Results ===");
    println!("Duration:     {:.2} s", elapsed);
    println!("Packet size:  {} bytes", packet_size);
    println!(
        "Packets:      {} sent, {} received, {} timeouts, {} CRC errors",
        packets_sent, packets_received, timeouts, crc_errors
    );

    if !rtts.is_empty() {
        let min_rtt = *rtts.iter().min().unwrap();
        let max_rtt = *rtts.iter().max().unwrap();
        let avg_rtt = rtts.iter().sum::<u64>() as f64 / rtts.len() as f64;

        println!(
            "RTT:          min={} ms, avg={:.1} ms, max={} ms",
            min_rtt, avg_rtt, max_rtt
        );
    }

    if elapsed > 0.0 {
        let speed_kbps = (bytes_transferred as f64 * 8.0) / elapsed / 1000.0;
        println!("Speed:        {:.2} kb/s", speed_kbps);
    }

    if packets_sent > 0 {
        let loss = ((packets_sent - packets_received) as f64 / packets_sent as f64) * 100.0;
        println!("Packet loss:  {:.1}%", loss);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_level(log::Level::Info)?;

    let args = Args::parse();

    if !args.server && !args.client {
        eprintln!("Error: specify --server or --client");
        std::process::exit(1);
    }

    if args.server {
        run_server(&args.address).await?;
    } else {
        run_client(&args.address, args.duration, args.size).await?;
    }

    Ok(())
}

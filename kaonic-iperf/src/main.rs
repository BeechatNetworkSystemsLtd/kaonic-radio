use clap::{Parser, Subcommand, ValueEnum};
use log::{error, info, warn};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_stream::StreamExt;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

use kaonic::{
    radio_client::RadioClient, ConfigurationRequest, RadioFrame, RadioModule,
    RadioPhyConfigOfdm, ReceiveRequest, TransmitRequest,
};

const DEFAULT_COMMD_ADDR: &str = "http://127.0.0.1:8080";
const DEFAULT_PACKET_SIZE: usize = 512;
const DEFAULT_DURATION_SECS: u64 = 10;
const DEFAULT_INTERVAL_MS: u64 = 100;
const MAX_PAYLOAD_SIZE: usize = 3600;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TestMode {
    Bandwidth,
    Latency,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Module {
    A,
    B,
}

impl From<Module> for RadioModule {
    fn from(m: Module) -> Self {
        match m {
            Module::A => RadioModule::ModuleA,
            Module::B => RadioModule::ModuleB,
        }
    }
}

impl From<Module> for i32 {
    fn from(m: Module) -> Self {
        match m {
            Module::A => RadioModule::ModuleA as i32,
            Module::B => RadioModule::ModuleB as i32,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "kaonic-iperf")]
#[command(about = "Network performance testing tool for Kaonic radio", long_about = None)]
struct Args {
    /// kaonic-commd gRPC address
    #[arg(short = 'a', long, default_value = DEFAULT_COMMD_ADDR)]
    address: String,

    /// Radio module to use
    #[arg(short = 'm', long, default_value = "a")]
    module: Module,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run as server (receiver)
    Server {
        /// Receive timeout in milliseconds (0 = infinite)
        #[arg(short = 't', long, default_value = "0")]
        timeout: u32,
    },
    /// Run as client (sender)
    Client {
        /// Test mode
        #[arg(short = 'M', long, default_value = "bandwidth")]
        mode: TestMode,

        /// Packet size in bytes (max 3600)
        #[arg(short = 's', long, default_value_t = DEFAULT_PACKET_SIZE)]
        size: usize,

        /// Test duration in seconds
        #[arg(short = 'd', long, default_value_t = DEFAULT_DURATION_SECS)]
        duration: u64,

        /// Interval between packets in milliseconds (for latency test)
        #[arg(short = 'i', long, default_value_t = DEFAULT_INTERVAL_MS)]
        interval: u64,

        /// Number of packets to send (overrides duration if set)
        #[arg(short = 'n', long)]
        count: Option<u64>,

        /// Wait for response (bidirectional test)
        #[arg(short = 'b', long)]
        bidirectional: bool,
    },
    /// Configure radio parameters
    Configure {
        /// Frequency in kHz
        #[arg(short = 'f', long, default_value = "915000")]
        freq: u32,

        /// Channel number
        #[arg(short = 'c', long, default_value = "0")]
        channel: u32,

        /// TX power in dBm
        #[arg(short = 'p', long, default_value = "14")]
        tx_power: u32,

        /// OFDM MCS (0-6)
        #[arg(long, default_value = "3")]
        mcs: u32,

        /// OFDM option (1-4)
        #[arg(long, default_value = "1")]
        opt: u32,
    },
}

#[derive(Debug, Default)]
struct Statistics {
    packets_sent: u64,
    packets_received: u64,
    bytes_sent: u64,
    bytes_received: u64,
    latencies_ms: Vec<u32>,
    start_time: Option<Instant>,
    end_time: Option<Instant>,
}

impl Statistics {
    fn new() -> Self {
        Self::default()
    }

    fn start(&mut self) {
        self.start_time = Some(Instant::now());
    }

    fn stop(&mut self) {
        self.end_time = Some(Instant::now());
    }

    fn add_sent(&mut self, bytes: u64) {
        self.packets_sent += 1;
        self.bytes_sent += bytes;
    }

    fn add_received(&mut self, bytes: u64, latency_ms: u32) {
        self.packets_received += 1;
        self.bytes_received += bytes;
        self.latencies_ms.push(latency_ms);
    }

    fn duration(&self) -> Duration {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => end.duration_since(start),
            (Some(start), None) => Instant::now().duration_since(start),
            _ => Duration::ZERO,
        }
    }

    fn throughput_bps(&self) -> f64 {
        let duration_secs = self.duration().as_secs_f64();
        if duration_secs > 0.0 {
            (self.bytes_sent as f64 * 8.0) / duration_secs
        } else {
            0.0
        }
    }

    fn rx_throughput_bps(&self) -> f64 {
        let duration_secs = self.duration().as_secs_f64();
        if duration_secs > 0.0 {
            (self.bytes_received as f64 * 8.0) / duration_secs
        } else {
            0.0
        }
    }

    fn avg_latency_ms(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            0.0
        } else {
            self.latencies_ms.iter().map(|&l| l as f64).sum::<f64>()
                / self.latencies_ms.len() as f64
        }
    }

    fn min_latency_ms(&self) -> u32 {
        self.latencies_ms.iter().copied().min().unwrap_or(0)
    }

    fn max_latency_ms(&self) -> u32 {
        self.latencies_ms.iter().copied().max().unwrap_or(0)
    }

    fn jitter_ms(&self) -> f64 {
        if self.latencies_ms.len() < 2 {
            return 0.0;
        }
        let avg = self.avg_latency_ms();
        let variance: f64 = self
            .latencies_ms
            .iter()
            .map(|&l| {
                let diff = l as f64 - avg;
                diff * diff
            })
            .sum::<f64>()
            / self.latencies_ms.len() as f64;
        variance.sqrt()
    }

    fn packet_loss_percent(&self) -> f64 {
        if self.packets_sent == 0 {
            0.0
        } else {
            ((self.packets_sent - self.packets_received) as f64 / self.packets_sent as f64) * 100.0
        }
    }

    fn print_summary(&self, mode: &str) {
        println!("\n========================================");
        println!("  Kaonic iPerf {} Summary", mode);
        println!("========================================");
        println!("Duration:        {:.2} s", self.duration().as_secs_f64());
        println!("Packets sent:    {}", self.packets_sent);
        println!("Packets recv:    {}", self.packets_received);
        println!("Bytes sent:      {} ({:.2} KB)", self.bytes_sent, self.bytes_sent as f64 / 1024.0);
        println!("Bytes recv:      {} ({:.2} KB)", self.bytes_received, self.bytes_received as f64 / 1024.0);
        println!("----------------------------------------");
        println!("TX Throughput:   {}", format_throughput(self.throughput_bps()));
        println!("RX Throughput:   {}", format_throughput(self.rx_throughput_bps()));
        println!("----------------------------------------");

        if !self.latencies_ms.is_empty() {
            println!("Latency (min):   {} ms", self.min_latency_ms());
            println!("Latency (avg):   {:.2} ms", self.avg_latency_ms());
            println!("Latency (max):   {} ms", self.max_latency_ms());
            println!("Jitter:          {:.2} ms", self.jitter_ms());
        }

        if self.packets_sent > 0 && self.packets_received < self.packets_sent {
            println!("Packet loss:     {:.2}%", self.packet_loss_percent());
        }
        println!("========================================\n");
    }
}

fn format_throughput(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.2} Mbps", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.2} Kbps", bps / 1_000.0)
    } else {
        format!("{:.2} bps", bps)
    }
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

const MAGIC_HEADER: [u8; 4] = [0x4B, 0x49, 0x50, 0x46]; // "KIPF" - Kaonic IPerf

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum PacketType {
    Data = 0x01,
    Ack = 0x02,
    Ping = 0x03,
    Pong = 0x04,
}

impl TryFrom<u8> for PacketType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(PacketType::Data),
            0x02 => Ok(PacketType::Ack),
            0x03 => Ok(PacketType::Ping),
            0x04 => Ok(PacketType::Pong),
            _ => Err(()),
        }
    }
}

fn create_test_packet(packet_type: PacketType, seq: u32, payload_size: usize) -> Vec<u8> {
    let mut packet = Vec::with_capacity(payload_size);

    // Header (12 bytes)
    packet.extend_from_slice(&MAGIC_HEADER);
    packet.push(packet_type as u8);
    packet.push(0); // reserved
    packet.push(0); // reserved
    packet.push(0); // reserved
    packet.extend_from_slice(&seq.to_le_bytes());

    // Timestamp (8 bytes)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    packet.extend_from_slice(&timestamp.to_le_bytes());

    // Padding to reach payload_size
    while packet.len() < payload_size {
        packet.push((packet.len() & 0xFF) as u8);
    }

    packet.truncate(payload_size);
    packet
}

fn parse_test_packet(data: &[u8]) -> Option<(PacketType, u32, u64)> {
    if data.len() < 20 {
        return None;
    }

    if data[0..4] != MAGIC_HEADER {
        return None;
    }

    let packet_type = PacketType::try_from(data[4]).ok()?;
    let seq = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let timestamp = u64::from_le_bytes([
        data[12], data[13], data[14], data[15],
        data[16], data[17], data[18], data[19],
    ]);

    Some((packet_type, seq, timestamp))
}

async fn run_server(
    address: &str,
    module: Module,
    recv_timeout: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting server mode on module {:?}", module);

    let mut client = RadioClient::connect(address.to_string()).await?;
    info!("Connected to kaonic-commd at {}", address);

    let mut stats = Statistics::new();
    stats.start();

    let request = ReceiveRequest {
        module: module.into(),
        timeout: recv_timeout,
    };

    let mut stream = client.receive_stream(request).await?.into_inner();

    println!("\n========================================");
    println!("  Kaonic iPerf Server");
    println!("========================================");
    println!("Listening on module {:?}...", module);
    println!("Press Ctrl+C to stop\n");

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("Shutdown signal received");
                break;
            }
            result = stream.next() => {
                match result {
                    Some(Ok(response)) => {
                        let frame_data = decode_frame(&response.frame.unwrap_or_default());
                        let frame_len = frame_data.len();

                        if let Some((packet_type, seq, timestamp)) = parse_test_packet(&frame_data) {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64;
                            let latency = (now_ms.saturating_sub(timestamp)) as u32;

                            stats.add_received(frame_len as u64, latency);

                            println!(
                                "[{}] Received {:?} seq={} size={} rssi={} dBm latency={} ms",
                                stats.packets_received,
                                packet_type,
                                seq,
                                frame_len,
                                response.rssi,
                                latency
                            );

                            // For ping packets, send pong response
                            if packet_type == PacketType::Ping {
                                let pong = create_test_packet(PacketType::Pong, seq, frame_len);
                                let tx_request = TransmitRequest {
                                    module: module.into(),
                                    frame: Some(encode_frame(&pong)),
                                };

                                match client.transmit(tx_request).await {
                                    Ok(resp) => {
                                        stats.add_sent(frame_len as u64);
                                        info!("Sent pong response seq={} latency={} ms", seq, resp.into_inner().latency);
                                    }
                                    Err(e) => {
                                        warn!("Failed to send pong: {}", e);
                                    }
                                }
                            }
                        } else {
                            stats.add_received(frame_len as u64, response.latency);
                            println!(
                                "[{}] Received raw frame size={} rssi={} dBm",
                                stats.packets_received,
                                frame_len,
                                response.rssi
                            );
                        }
                    }
                    Some(Err(e)) => {
                        error!("Receive error: {}", e);
                    }
                    None => {
                        info!("Stream ended");
                        break;
                    }
                }
            }
        }
    }

    stats.stop();
    stats.print_summary("Server");

    Ok(())
}

async fn run_client(
    address: &str,
    module: Module,
    mode: TestMode,
    packet_size: usize,
    duration_secs: u64,
    interval_ms: u64,
    packet_count: Option<u64>,
    bidirectional: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting client mode on module {:?}", module);

    let packet_size = packet_size.min(MAX_PAYLOAD_SIZE);

    let mut client = RadioClient::connect(address.to_string()).await?;
    info!("Connected to kaonic-commd at {}", address);

    let mut stats = Statistics::new();

    println!("\n========================================");
    println!("  Kaonic iPerf Client");
    println!("========================================");
    println!("Mode:            {:?}", mode);
    println!("Module:          {:?}", module);
    println!("Packet size:     {} bytes", packet_size);
    if let Some(count) = packet_count {
        println!("Packet count:    {}", count);
    } else {
        println!("Duration:        {} s", duration_secs);
    }
    if bidirectional {
        println!("Bidirectional:   yes");
    }
    println!("----------------------------------------\n");

    // Start receive stream if bidirectional
    let mut rx_stream = if bidirectional {
        let request = ReceiveRequest {
            module: module.into(),
            timeout: 5000, // 5 second timeout for responses
        };
        Some(client.receive_stream(request).await?.into_inner())
    } else {
        None
    };

    stats.start();

    let end_time = Instant::now() + Duration::from_secs(duration_secs);
    let mut seq: u32 = 0;
    let interval = Duration::from_millis(interval_ms);

    let packet_type = match mode {
        TestMode::Bandwidth => PacketType::Data,
        TestMode::Latency => PacketType::Ping,
    };

    loop {
        // Check termination conditions
        if let Some(count) = packet_count {
            if stats.packets_sent >= count {
                break;
            }
        } else if Instant::now() >= end_time {
            break;
        }

        // Create and send test packet
        let packet = create_test_packet(packet_type, seq, packet_size);
        let tx_request = TransmitRequest {
            module: module.into(),
            frame: Some(encode_frame(&packet)),
        };

        match client.transmit(tx_request).await {
            Ok(response) => {
                let tx_latency = response.into_inner().latency;
                stats.add_sent(packet_size as u64);

                match mode {
                    TestMode::Bandwidth => {
                        if stats.packets_sent % 10 == 0 {
                            println!(
                                "[{}] Sent {} bytes, tx_latency={} ms, throughput={}",
                                stats.packets_sent,
                                packet_size,
                                tx_latency,
                                format_throughput(stats.throughput_bps())
                            );
                        }
                    }
                    TestMode::Latency => {
                        println!(
                            "[{}] Sent ping seq={} size={} tx_latency={} ms",
                            stats.packets_sent, seq, packet_size, tx_latency
                        );
                    }
                }
            }
            Err(e) => {
                error!("Transmit error: {}", e);
            }
        }

        // Wait for response if bidirectional/latency mode
        if bidirectional || matches!(mode, TestMode::Latency) {
            if let Some(ref mut stream) = rx_stream {
                match timeout(Duration::from_millis(2000), stream.next()).await {
                    Ok(Some(Ok(response))) => {
                        let frame_data = decode_frame(&response.frame.unwrap_or_default());

                        if let Some((pkt_type, pkt_seq, timestamp)) = parse_test_packet(&frame_data) {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64;
                            let rtt = (now_ms.saturating_sub(timestamp)) as u32;

                            stats.add_received(frame_data.len() as u64, rtt);

                            println!(
                                "  <- Received {:?} seq={} rtt={} ms rssi={} dBm",
                                pkt_type, pkt_seq, rtt, response.rssi
                            );
                        }
                    }
                    Ok(Some(Err(e))) => {
                        warn!("Receive error: {}", e);
                    }
                    Ok(None) => {
                        info!("Receive stream ended");
                    }
                    Err(_) => {
                        if matches!(mode, TestMode::Latency) {
                            warn!("Timeout waiting for pong response");
                        }
                    }
                }
            }
        }

        seq = seq.wrapping_add(1);

        // Apply interval delay
        tokio::time::sleep(interval).await;
    }

    stats.stop();
    stats.print_summary("Client");

    Ok(())
}

async fn run_configure(
    address: &str,
    module: Module,
    freq: u32,
    channel: u32,
    tx_power: u32,
    mcs: u32,
    opt: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Configuring radio module {:?}", module);

    let mut client = RadioClient::connect(address.to_string()).await?;
    info!("Connected to kaonic-commd at {}", address);

    let request = ConfigurationRequest {
        module: module.into(),
        freq,
        channel,
        channel_spacing: 200, // 200 kHz default
        tx_power,
        phy_config: Some(kaonic::configuration_request::PhyConfig::Ofdm(
            RadioPhyConfigOfdm { mcs, opt },
        )),
        qos: None,
        bandwidth_filter: 0,
    };

    client.configure(request).await?;

    println!("\n========================================");
    println!("  Radio Configuration");
    println!("========================================");
    println!("Module:          {:?}", module);
    println!("Frequency:       {} kHz ({:.3} MHz)", freq, freq as f64 / 1000.0);
    println!("Channel:         {}", channel);
    println!("TX Power:        {} dBm", tx_power);
    println!("PHY:             OFDM MCS={} OPT={}", mcs, opt);
    println!("========================================\n");
    println!("Configuration applied successfully.");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_level(log::Level::Info)?;

    let args = Args::parse();

    match args.command {
        Commands::Server { timeout } => {
            run_server(&args.address, args.module, timeout).await?;
        }
        Commands::Client {
            mode,
            size,
            duration,
            interval,
            count,
            bidirectional,
        } => {
            run_client(
                &args.address,
                args.module,
                mode,
                size,
                duration,
                interval,
                count,
                bidirectional,
            )
            .await?;
        }
        Commands::Configure {
            freq,
            channel,
            tx_power,
            mcs,
            opt,
        } => {
            run_configure(&args.address, args.module, freq, channel, tx_power, mcs, opt).await?;
        }
    }

    Ok(())
}

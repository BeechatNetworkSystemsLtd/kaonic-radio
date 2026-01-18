use crate::grpc_client::{GrpcClient, TxTarget};
use crate::ui::AppState;
use parking_lot::Mutex;
use crate::kaonic::RadioModule;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct IperfClientHandle {
    pub thread: thread::JoinHandle<()>,
}

pub struct IperfServerHandle {
    pub thread: thread::JoinHandle<()>,
}

impl IperfClientHandle {
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

impl IperfServerHandle {
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

// Payload layout (big-endian): [ key: u32 | client_id: u32 | seq: u64 | ts_nanos: u64 | payload... ]
const IPERF_HDR_LEN: usize = 4 + 4 + 8 + 8;

pub fn start_client(
    client: Arc<Mutex<GrpcClient>>,
    state: Arc<Mutex<AppState>>,
    duration_secs: u64,
    payload_size: usize,
    interval_ms: u64,
    key: u32,
) -> IperfClientHandle {
    let thread = thread::spawn(move || {
        let start = Instant::now();
        let mut seq: u64 = 0;
        let mut sent_bytes: u64 = 0;
        let mut packets: u64 = 0;
        let payload_size = payload_size.max(IPERF_HDR_LEN);
        // generate a client id (lower 32 bits of current time) to identify replies
        let client_id: u32 = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64 & 0xFFFF_FFFF) as u32;

        let mut pending: HashMap<u64, Instant> = HashMap::new();
        let mut last_sample_time = Instant::now();
        let mut last_sent_bytes: u64 = 0;

        // subscribe to broadcast receive events so we can match replies
        let mut rx_recv = client.lock().rx_subscribe();

        while Instant::now().duration_since(start).as_secs() < duration_secs {
            // check if user cancelled
            {
                let s = state.lock();
                if !s.iperf_client_running {
                    break;
                }
            }

            // build payload
            let now_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;

            let mut payload = Vec::with_capacity(payload_size);
            payload.extend_from_slice(&key.to_be_bytes());
            payload.extend_from_slice(&client_id.to_be_bytes());
            payload.extend_from_slice(&seq.to_be_bytes());
            payload.extend_from_slice(&now_nanos.to_be_bytes());
            if payload.len() < payload_size {
                payload.resize(payload_size, 0);
            }

            // Transmit using central TX queue (Network target) with a oneshot reply
            match client.lock().tx_send_blocking(TxTarget::Network, payload.clone(), Some(5000)) {
                Ok(_latency) => {
                    // latency available if needed
                }
                Err(e) => {
                    let mut s = state.lock();
                    s.iperf_output.push_str(&format!("tx error: {}\n", e));
                }
            }

            sent_bytes += payload.len() as u64;
            packets += 1;
            pending.insert(seq, Instant::now());

            // throughput accounting: compute kB/s over short intervals
            let now_sample = Instant::now();
            let dt = now_sample.duration_since(last_sample_time).as_secs_f64();
            if dt >= 0.5 {
                let db = sent_bytes.saturating_sub(last_sent_bytes) as f64;
                let kbps = if dt > 0.0 { (db / 1024.0) / dt } else { 0.0 };
                let mut s = state.lock();
                s.iperf_client_kbps = kbps;
                last_sample_time = now_sample;
                last_sent_bytes = sent_bytes;
            }

            // update status
            {
                let mut s = state.lock();
                s.iperf_status = format!("Client: sent {} packets ({} bytes)", packets, sent_bytes);
            }

            // Drain any available responses from the broadcast receiver (non-blocking)
            use tokio::sync::broadcast::error::TryRecvError;
            let mut output_lines: Vec<String> = Vec::new();
            loop {
                match rx_recv.try_recv() {
                    Ok(ev) => {
                        if ev.frame_data.len() >= IPERF_HDR_LEN {
                            let k = u32::from_be_bytes([ev.frame_data[0], ev.frame_data[1], ev.frame_data[2], ev.frame_data[3]]);
                            if k != key { continue; }
                            let resp_client_id = u32::from_be_bytes([ev.frame_data[4], ev.frame_data[5], ev.frame_data[6], ev.frame_data[7]]);
                            if resp_client_id != client_id { continue; }
                            let seq_bytes: [u8; 8] = ev.frame_data[8..16].try_into().unwrap();
                            let resp_seq = u64::from_be_bytes(seq_bytes);
                            if let Some(sent_t) = pending.remove(&resp_seq) {
                                let rtt = sent_t.elapsed().as_secs_f64() * 1000.0;
                                output_lines.push(format!("seq={} rtt={:.2} ms", resp_seq, rtt));
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(_)) => continue,
                    Err(TryRecvError::Closed) => break,
                }
            }

            if !output_lines.is_empty() {
                let mut s = state.lock();
                for l in output_lines {
                    s.iperf_output.push_str(&format!("{}\n", l));
                }
            }

            seq = seq.wrapping_add(1);
            thread::sleep(Duration::from_millis(interval_ms.max(1)));
        }

        let mut s = state.lock();
        s.iperf_status = format!("Client finished: {} packets, {} bytes", packets, sent_bytes);
        s.iperf_client_running = false;
    });

    IperfClientHandle { thread }
}

pub fn start_server_monitor(
    client: Arc<Mutex<GrpcClient>>,
    state: Arc<Mutex<AppState>>,
    key: u32,
) -> IperfServerHandle {
    let thread = thread::spawn(move || {
        let mut last_index: usize = 0;
        let mut total_packets: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut last_sample_time = Instant::now();
        let mut last_total_bytes: u64 = 0;

        while {
            let s = state.lock();
            s.iperf_server_running
        } {
            // collect new events to process without holding lock during network sends
            let events_to_process = {
                let s = state.lock();
                if last_index >= s.rx_events.len() {
                    Vec::new()
                } else {
                    s.rx_events[last_index..].to_vec()
                }
            };

            if !events_to_process.is_empty() {
                for ev in events_to_process.iter() {
                        if ev.frame_data.len() >= IPERF_HDR_LEN {
                        let k = u32::from_be_bytes([ev.frame_data[0], ev.frame_data[1], ev.frame_data[2], ev.frame_data[3]]);
                        if k != key {
                            continue;
                        }
                        // extract client id and seq from incoming packet
                        let incoming_client_id = u32::from_be_bytes([ev.frame_data[4], ev.frame_data[5], ev.frame_data[6], ev.frame_data[7]]);
                        let seq_bytes: [u8; 8] = ev.frame_data[8..16].try_into().unwrap();
                        let seq = u64::from_be_bytes(seq_bytes);

                        // Build small response: echo key + client_id + seq + server ts
                        let server_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
                        let mut resp: Vec<u8> = Vec::with_capacity(IPERF_HDR_LEN);
                        resp.extend_from_slice(&key.to_be_bytes());
                        resp.extend_from_slice(&incoming_client_id.to_be_bytes());
                        resp.extend_from_slice(&seq.to_be_bytes());
                        resp.extend_from_slice(&server_ts.to_be_bytes());

                        // send response using the same radio module that received the packet
                        let module = if ev.module == 0 { RadioModule::ModuleA } else { RadioModule::ModuleB };
                        match client.lock().tx_send_blocking(TxTarget::Radio(module), resp, Some(5000)) {
                            Ok(_lat) => { /* transmitted */ }
                            Err(e) => {
                                let mut s = state.lock();
                                s.iperf_output.push_str(&format!("server tx err: {}\n", e));
                            }
                        }

                        total_packets += 1;
                        total_bytes += ev.frame_data.len() as u64;
                    }
                    last_index += 1;
                }

                // update status and throughput
                let now = Instant::now();
                let dt = now.duration_since(last_sample_time).as_secs_f64();
                if dt > 0.0 {
                    let db = total_bytes.saturating_sub(last_total_bytes) as f64;
                    let kbps = (db / 1024.0) / dt;
                    let mut s = state.lock();
                    s.iperf_status = format!("Server: processed {} pkts, {} bytes", total_packets, total_bytes);
                    s.iperf_server_kbps = kbps;
                } else {
                    let mut s = state.lock();
                    s.iperf_status = format!("Server: processed {} pkts, {} bytes", total_packets, total_bytes);
                }
                last_sample_time = now;
                last_total_bytes = total_bytes;
            }

            thread::sleep(Duration::from_millis(200));
        }

        let mut s = state.lock();
        s.iperf_status = "Server stopped".to_string();
        s.iperf_server_running = false;
    });

    IperfServerHandle { thread }
}

use crate::kaonic::{
    device_client::DeviceClient, radio_client::RadioClient, ConfigurationRequest, Empty,
    QoSConfig, RadioFrame, RadioModule, ReceiveRequest, TransmitRequest,
};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

pub struct GrpcClient {
    runtime: Arc<Runtime>,
    server_addr: String,
}

#[derive(Clone, Debug)]
pub struct ReceiveEvent {
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub module: i32,
    pub frame_data: Vec<u8>,
    pub rssi: i32,
    pub latency: u32,
}

impl GrpcClient {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            server_addr: "http://127.0.0.1:50010".to_string(),
        }
    }

    pub fn set_server_addr(&mut self, addr: String) {
        self.server_addr = addr;
    }

    #[allow(dead_code)]
    pub fn get_server_addr(&self) -> String {
        self.server_addr.clone()
    }

    pub async fn connect_device(&self) -> Result<DeviceClient<tonic::transport::Channel>, String> {
        DeviceClient::connect(self.server_addr.clone())
            .await
            .map_err(|e| format!("Failed to connect: {}", e))
    }

    pub async fn connect_radio(&self) -> Result<RadioClient<tonic::transport::Channel>, String> {
        RadioClient::connect(self.server_addr.clone())
            .await
            .map_err(|e| format!("Failed to connect: {}", e))
    }

    pub fn get_device_info(&self) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut client = self.connect_device().await?;
            client
                .get_info(tonic::Request::new(Empty {}))
                .await
                .map_err(|e| format!("Failed to get device info: {}", e))?;
            Ok(())
        })
    }

    pub fn configure_radio(
        &self,
        module: RadioModule,
        freq: u32,
        channel: u32,
        channel_spacing: u32,
        tx_power: u32,
        phy_config: Option<crate::kaonic::configuration_request::PhyConfig>,
        qos_enabled: bool,
        qos_config: QoSConfig,
    ) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut client = self.connect_radio().await?;

            let qos = if qos_enabled {
                Some(QoSConfig {
                    enabled: qos_config.enabled,
                    adaptive_modulation: qos_config.adaptive_modulation,
                    adaptive_tx_power: qos_config.adaptive_tx_power,
                    adaptive_backoff: qos_config.adaptive_backoff,
                    cca_threshold: qos_config.cca_threshold,
                })
            } else {
                None
            };

            let request = ConfigurationRequest {
                module: module as i32,
                freq,
                channel,
                channel_spacing,
                tx_power,
                phy_config,
                qos,
            };

            client
                .configure(tonic::Request::new(request))
                .await
                .map_err(|e| format!("Failed to configure: {}", e))?;

            Ok(())
        })
    }

    pub fn transmit_frame(
        &self,
        module: RadioModule,
        data: Vec<u8>,
    ) -> Result<u32, String> {
        self.runtime.block_on(async {
            let mut client = self.connect_radio().await?;

            let frame = RadioFrame {
                data: data.iter().map(|&b| b as u32).collect(),
                length: data.len() as u32,
            };

            let request = TransmitRequest {
                module: module as i32,
                frame: Some(frame),
            };

            let response = client
                .transmit(tonic::Request::new(request))
                .await
                .map_err(|e| format!("Failed to transmit: {}", e))?;

            Ok(response.into_inner().latency)
        })
    }

    pub fn start_receive_stream(
        &self,
        module: RadioModule,
        rx: mpsc::UnboundedSender<ReceiveEvent>,
    ) {
        let server_addr = self.server_addr.clone();

        self.runtime.spawn(async move {
            loop {
                match RadioClient::connect(server_addr.clone()).await {
                    Ok(mut client) => {
                        let request = ReceiveRequest {
                            module: module as i32,
                            timeout: 1000,
                        };

                        match client.receive_stream(tonic::Request::new(request)).await {
                            Ok(response) => {
                                let mut stream = response.into_inner();

                                while let Some(result) = stream.next().await {
                                    match result {
                                        Ok(rx_response) => {
                                            let frame_data = if let Some(frame) = rx_response.frame {
                                                frame
                                                    .data
                                                    .iter()
                                                    .take(frame.length as usize)
                                                    .map(|&w| w as u8)
                                                    .collect()
                                            } else {
                                                Vec::new()
                                            };

                                            let event = ReceiveEvent {
                                                timestamp: chrono::Local::now(),
                                                module: rx_response.module,
                                                frame_data,
                                                rssi: rx_response.rssi,
                                                latency: rx_response.latency,
                                            };

                                            if rx.send(event).is_err() {
                                                return; // Channel closed, stop stream
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("Stream error: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Failed to start receive stream: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to connect: {}", e);
                    }
                }

                // Wait before reconnecting
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });
    }
}

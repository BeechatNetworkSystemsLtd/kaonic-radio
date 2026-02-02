use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::error::Error;

use crate::kaonic::{ConfigurationRequest, RadioModule, RadioPhyConfigOfdm, RadioPhyConfigQpsk, configuration_request::PhyConfig};

#[derive(Debug)]
pub struct IperfConfig {
    pub duration: u64,
    pub payload_size: usize,
    pub timeout: u64,
    pub ip: Option<String>,
    pub module: i32, // RadioModule enum value
}

impl Default for IperfConfig {
    fn default() -> Self {
        IperfConfig {
            duration: 10,
            payload_size: 2047,
            timeout: 10,
            ip: None,
            module: 0, // MODULE_A
        }
    }
}

#[derive(Debug, Default)]
pub struct Config {
    // Radios are represented as protobuf ConfigurationRequest objects directly
    pub radios: Vec<ConfigurationRequest>,
    pub iperf: IperfConfig,
    pub modulation: HashMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct IperfPartial {
    duration: Option<u64>,
    payload_size: Option<usize>,
    timeout: Option<u64>,
    ip: Option<String>,
    module: Option<i64>,
}

/// Loads configuration from the given TOML file path and maps radio-* sections to protobufs.
pub fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let s = fs::read_to_string(path)?;
    let val: toml::Value = toml::from_str(&s)?;
    let table = val.as_table().ok_or("config is not a table")?;

    // Modulation presets (parse first so presets can be applied while creating radios)
    let mut modulation: HashMap<String, toml::Value> = HashMap::new();
    if let Some(mv) = table.get("modulation").and_then(|v| v.as_table()) {
        for (k, v) in mv.iter() {
            modulation.insert(k.clone(), v.clone());
        }
    }

    // Collect radio keys in sorted order for deterministic mapping
    let mut radio_keys: Vec<String> = table.keys().filter(|k| k.starts_with("radio-")).cloned().collect();
    radio_keys.sort();

    let mut radios_proto: Vec<ConfigurationRequest> = Vec::new();
    for (i, key) in radio_keys.iter().enumerate() {
        if let Some(v) = table.get(key) {
            if let Some(rtab) = v.as_table() {
                let mut req = ConfigurationRequest::default();

                // Map module index to enum
                req.module = match i {
                    0 => RadioModule::ModuleA as i32,
                    1 => RadioModule::ModuleB as i32,
                    _ => RadioModule::ModuleA as i32,
                };

                if let Some(freq) = rtab.get("freq").and_then(|x| x.as_integer()) {
                    req.freq = freq as u32;
                }
                if let Some(ch) = rtab.get("channel").and_then(|x| x.as_integer()) {
                    req.channel = ch as u32;
                }
                if let Some(cs) = rtab.get("channel_spacing").and_then(|x| x.as_integer()) {
                    req.channel_spacing = cs as u32;
                }
                if let Some(tp) = rtab.get("tx_power").and_then(|x| x.as_integer()) {
                    req.tx_power = tp as u32;
                }

                // Apply modulation preset in place if specified
                if let Some(mod_name) = rtab.get("modulation").and_then(|x| x.as_str()) {
                    if let Some(preset) = modulation.get(mod_name) {
                        if let Some(t) = preset.get("type").and_then(|v| v.as_str()) {
                            match t {
                                "ofdm" => {
                                    let mcs = preset.get("mcs").and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                                    let opt = preset.get("opt").and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                                    let ofdm = RadioPhyConfigOfdm { mcs, opt };
                                    req.phy_config = Some(PhyConfig::Ofdm(ofdm));
                                }
                                "qpsk" => {
                                    let chip = preset.get("chip_freq").and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                                    let rate = preset.get("rate").and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                                    let qpsk = RadioPhyConfigQpsk { chip_freq: chip, rate_mode: rate };
                                    req.phy_config = Some(PhyConfig::Qpsk(qpsk));
                                }
                                _ => {}
                            }
                        }
                    }
                }

                radios_proto.push(req);
            }
        }
    }

    // Parse iperf section
    let iperf = if let Some(v) = table.get("iperf") {
        let mut d = IperfConfig::default();
        if let Ok(partial) = v.clone().try_into::<IperfPartial>() {
            if let Some(x) = partial.duration { d.duration = x; }
            if let Some(x) = partial.payload_size { d.payload_size = x; }
            if let Some(x) = partial.timeout { d.timeout = x; }
            if let Some(x) = partial.ip { d.ip = Some(x); }
            if let Some(m) = partial.module {
                d.module = match m {
                    0 => RadioModule::ModuleA as i32,
                    1 => RadioModule::ModuleB as i32,
                    _ => RadioModule::ModuleA as i32,
                };
            }
        }
        d
    } else {
        IperfConfig::default()
    };

    // Modulation presets
    let mut modulation: HashMap<String, toml::Value> = HashMap::new();
    if let Some(mv) = table.get("modulation").and_then(|v| v.as_table()) {
        for (k, v) in mv.iter() {
            modulation.insert(k.clone(), v.clone());
        }
    }


    Ok(Config { radios: radios_proto, iperf, modulation })
}

use super::FactoryTest;
use std::collections::HashMap;
use std::fs;
use std::process::Command;

pub struct I2cDevicesTest;

#[tonic::async_trait]
impl FactoryTest for I2cDevicesTest {
    fn name(&self) -> &str {
        "I2C Devices Test"
    }

    fn description(&self) -> &str {
        "Check if all expected I2C devices are present and responding"
    }

    async fn execute(&self) -> Result<String, String> {
        let mut i2c_info = Vec::new();
        let mut buses_scanned = 0;

        // Define expected I2C devices for your PCB
        // Format: (address, device_name) - no specific bus required
        let expected_devices = self.get_expected_i2c_devices();
        let total_expected = expected_devices.len();

        // Get all available I2C buses
        let i2c_buses = self.get_i2c_buses().await?;

        if i2c_buses.is_empty() {
            return Err("No I2C buses found on system".to_string());
        }

        // Collect all detected devices across all buses
        let mut all_detected_devices = HashMap::new(); // address -> (bus, device_info)

        for bus in i2c_buses {
            buses_scanned += 1;

            match self.scan_i2c_bus(bus).await {
                Ok(detected_devices) => {
                    for addr in detected_devices {
                        all_detected_devices.insert(addr, bus);
                    }
                }
                Err(e) => {
                    i2c_info.push(format!("Bus {}: scan failed ({})", bus, e));
                }
            }
        }

        // Check which expected devices were found
        let mut found_devices = Vec::new();
        let mut missing_devices = Vec::new();
        let mut total_found = 0;

        for (addr, name) in expected_devices {
            if let Some(bus) = all_detected_devices.get(&addr) {
                total_found += 1;
                found_devices.push(format!("{}(0x{:02x})@bus{}", name, addr, bus));
            } else {
                missing_devices.push(format!("{}(0x{:02x})", name, addr));
            }
        }

        // Build result information
        if !found_devices.is_empty() {
            i2c_info.push(format!("Found: {}", found_devices.join(", ")));
        }

        if !missing_devices.is_empty() {
            i2c_info.push(format!("Missing: {}", missing_devices.join(", ")));
        }

        // Generate summary
        let success_rate = if total_expected > 0 {
            (total_found * 100) / total_expected
        } else {
            100
        };

        let summary = format!(
            "I2C scan: {}/{} expected devices found ({}%) across {} buses",
            total_found, total_expected, success_rate, buses_scanned
        );

        // Determine if test passed
        if total_expected > 0 && total_found < total_expected {
            let missing_count = total_expected - total_found;
            return Err(format!(
                "{} | {} | Missing {} critical I2C devices",
                summary,
                i2c_info.join(" | "),
                missing_count
            ));
        }

        if i2c_info.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{} | {}", summary, i2c_info.join(" | ")))
        }
    }
}

impl I2cDevicesTest {
    fn get_expected_i2c_devices(&self) -> Vec<(u8, &'static str)> {
        // Define your expected I2C devices here
        // Format: (device_address, device_name) - bus-agnostic
        vec![
            (0x21, "GPIO Expansion"),
            (0x21, "GPIO Expansion"),
            (0x48, "TEMP Sens"),
            (0x49, "TEMP Sens"),
            (0x33, "PMIC"),
            (0x6b, "BQ25792 Charger"),
        ]
    }

    async fn get_i2c_buses(&self) -> Result<Vec<u8>, String> {
        let mut buses = Vec::new();

        // Method 1: Check /dev/i2c-* devices
        if let Ok(entries) = fs::read_dir("/dev") {
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("i2c-") {
                        if let Ok(bus_num) = name[4..].parse::<u8>() {
                            buses.push(bus_num);
                        }
                    }
                }
            }
        }

        // Method 2: Check /sys/class/i2c-adapter/
        if buses.is_empty() {
            if let Ok(entries) = fs::read_dir("/sys/class/i2c-adapter") {
                for entry in entries.filter_map(|e| e.ok()) {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.starts_with("i2c-") {
                            if let Ok(bus_num) = name[4..].parse::<u8>() {
                                buses.push(bus_num);
                            }
                        }
                    }
                }
            }
        }

        buses.sort();

        if buses.is_empty() {
            Err("No I2C buses found".to_string())
        } else {
            Ok(buses)
        }
    }

    async fn scan_i2c_bus(&self, bus: u8) -> Result<Vec<u8>, String> {
        // Try i2cdetect command first (most reliable)
        if let Ok(output) = Command::new("i2cdetect")
            .args(&["-y", "-r", &bus.to_string()])
            .output()
        {
            if output.status.success() {
                return self.parse_i2cdetect_output(&String::from_utf8_lossy(&output.stdout));
            }
        }

        // Fallback: Try to scan via sysfs
        self.scan_i2c_bus_sysfs(bus).await
    }

    fn parse_i2cdetect_output(&self, output: &str) -> Result<Vec<u8>, String> {
        let mut devices = Vec::new();

        for (row_index, line) in output.lines().skip(1).enumerate() {
            // Skip header line, row_index starts at 0 for first data row
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 1 {
                // Skip the first element (row address like "00:", "10:", etc.)
                for (col_index, addr_str) in parts.iter().skip(1).enumerate() {
                    if *addr_str != "--" {
                        if addr_str.len() == 2 {
                            // Normal hex address like "48", "21", etc.
                            if let Ok(addr) = u8::from_str_radix(addr_str, 16) {
                                devices.push(addr);
                            } else if *addr_str == "UU" {
                                // Device is in use by kernel driver
                                // Calculate address from row and column position
                                let row_base = row_index * 16; // Each row represents 16 addresses
                                let addr = row_base + col_index;
                                if addr <= 0x7F {
                                    // Valid I2C 7-bit address range
                                    devices.push(addr as u8);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(devices)
    }

    async fn scan_i2c_bus_sysfs(&self, bus: u8) -> Result<Vec<u8>, String> {
        let mut devices = Vec::new();
        let bus_path = format!("/sys/bus/i2c/devices/{}-*", bus);

        // Use shell globbing to find devices
        if let Ok(output) = Command::new("sh")
            .args(&["-c", &format!("ls -d {} 2>/dev/null", bus_path)])
            .output()
        {
            let paths = String::from_utf8_lossy(&output.stdout);
            for path in paths.lines() {
                if let Some(device_name) = path.split('/').last() {
                    if let Some(addr_str) = device_name.split('-').nth(1) {
                        if let Ok(addr) = u8::from_str_radix(addr_str, 16) {
                            devices.push(addr);
                        }
                    }
                }
            }
        }

        Ok(devices)
    }
}

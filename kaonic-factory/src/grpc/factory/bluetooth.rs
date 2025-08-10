use std::fs;
use std::process::Command;
use super::FactoryTest;

pub struct BluetoothInitTest;

#[tonic::async_trait]
impl FactoryTest for BluetoothInitTest {
    fn name(&self) -> &str {
        "Bluetooth Module Initialization Test"
    }
    
    fn description(&self) -> &str {
        "Verify Bluetooth module is properly initialized by Linux kernel"
    }
    
    async fn execute(&self) -> Result<String, String> {
        // Check if Bluetooth kernel modules are loaded
        let lsmod_output = Command::new("lsmod")
            .output()
            .map_err(|e| format!("Failed to execute lsmod: {}", e))?;

        let modules = String::from_utf8_lossy(&lsmod_output.stdout);
        let bt_modules = ["bluetooth", "btusb", "hci_uart"];
        let mut loaded_modules = Vec::new();

        for module in bt_modules.iter() {
            if modules.contains(module) {
                loaded_modules.push(*module);
            }
        }

        if loaded_modules.is_empty() {
            return Err("No Bluetooth kernel modules found loaded".to_string());
        }

        // Check if Bluetooth devices are present in /sys/class/bluetooth
        let bt_devices = match fs::read_dir("/sys/class/bluetooth") {
            Ok(entries) => {
                let devices: Vec<_> = entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| name.starts_with("hci"))
                    .collect();
                devices
            }
            Err(_) => Vec::new(),
        };

        // Use hciconfig to check Bluetooth controller status
        let hciconfig_output = Command::new("hciconfig")
            .output()
            .map_err(|e| format!("Failed to execute hciconfig: {}", e))?;

        if !hciconfig_output.status.success() {
            return Err(
                "hciconfig command failed - Bluetooth subsystem may not be available".to_string(),
            );
        }

        let hci_info = String::from_utf8_lossy(&hciconfig_output.stdout);

        if hci_info.is_empty() || hci_info.contains("No such device") {
            return Err("No Bluetooth controllers found".to_string());
        }

        // Check for UP and RUNNING status
        let controllers: Vec<&str> = hci_info
            .lines()
            .filter(|line| line.starts_with("hci"))
            .collect();

        if controllers.is_empty() {
            return Err("No Bluetooth controllers detected".to_string());
        }

        let mut controller_status = Vec::new();
        for controller in controllers {
            let controller_name = controller.split(':').next().unwrap_or("unknown");
            let is_up = hci_info.contains("UP RUNNING");
            let status = if is_up { "UP RUNNING" } else { "DOWN" };
            controller_status.push(format!("{}: {}", controller_name, status));
        }

        // Try to get Bluetooth version info
        let version_info = match Command::new("bluetoothctl").args(&["--version"]).output() {
            Ok(output) => {
                if output.status.success() {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        let mut result_parts = vec![
            format!("Loaded kernel modules: {}", loaded_modules.join(", ")),
            format!("Controllers: {}", controller_status.join(", ")),
        ];

        if !bt_devices.is_empty() {
            result_parts.push(format!(
                "Devices in /sys/class/bluetooth: {}",
                bt_devices.join(", ")
            ));
        }

        if let Some(version) = version_info {
            result_parts.push(format!("BlueZ version: {}", version));
        }

        Ok(result_parts.join(" | "))
    }
}
use super::FactoryTest;
use std::fs;
use std::process::Command;

pub struct WiFiInitTest;

#[tonic::async_trait]
impl FactoryTest for WiFiInitTest {
    fn name(&self) -> &str {
        "WiFi Module Initialization Test"
    }

    fn description(&self) -> &str {
        "Verify WiFi module is properly initialized by Linux kernel (read-only)"
    }

    async fn execute(&self) -> Result<String, String> {
        // Check if WiFi kernel modules are loaded
        let lsmod_output = Command::new("lsmod")
            .output()
            .map_err(|e| format!("Failed to execute lsmod: {}", e))?;

        let modules = String::from_utf8_lossy(&lsmod_output.stdout);
        let wifi_modules = ["cfg80211", "mac80211", "brcmfmac"];
        let mut loaded_modules = Vec::new();

        for module in wifi_modules.iter() {
            if modules.contains(module) {
                loaded_modules.push(*module);
            }
        }

        if loaded_modules.is_empty() {
            return Err("No WiFi kernel modules found loaded".to_string());
        }

        // Check for wireless interfaces in /sys/class/net
        let net_interfaces = match fs::read_dir("/sys/class/net") {
            Ok(entries) => {
                let wireless_interfaces: Vec<_> = entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        // Check if it's a wireless interface by looking for wireless directory
                        let wireless_path = format!("/sys/class/net/{}/wireless", name);
                        if fs::metadata(wireless_path).is_ok() {
                            Some(name)
                        } else {
                            None
                        }
                    })
                    .collect();
                wireless_interfaces
            }
            Err(_) => Vec::new(),
        };

        if net_interfaces.is_empty() {
            return Err("No wireless network interfaces found".to_string());
        }

        // Use iw to check WiFi interface information (read-only)
        let iw_output = Command::new("iw")
            .args(&["dev"])
            .output()
            .map_err(|e| format!("Failed to execute iw: {}", e))?;

        if !iw_output.status.success() {
            return Err("iw command failed - wireless subsystem may not be available".to_string());
        }

        let iw_info = String::from_utf8_lossy(&iw_output.stdout);

        if iw_info.is_empty() {
            return Err("No wireless devices found by iw".to_string());
        }

        // Extract interface information without changing state
        let mut interface_info = Vec::new();
        let lines: Vec<&str> = iw_info.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i].trim();
            if line.starts_with("Interface") {
                let interface_name = line.split_whitespace().nth(1).unwrap_or("unknown");

                // Look for type and channel info in following lines
                let mut interface_type = "unknown";
                let mut channel_info = None;

                for j in i + 1..std::cmp::min(i + 10, lines.len()) {
                    let info_line = lines[j].trim();
                    if info_line.starts_with("type") {
                        interface_type = info_line.split_whitespace().nth(1).unwrap_or("unknown");
                    } else if info_line.starts_with("channel") {
                        channel_info = Some(info_line);
                    } else if info_line.starts_with("Interface") {
                        break;
                    }
                }

                let mut info = format!("{}: {}", interface_name, interface_type);
                if let Some(ch) = channel_info {
                    info.push_str(&format!(" ({})", ch));
                }
                interface_info.push(info);
            }
            i += 1;
        }

        // Check if NetworkManager or wpa_supplicant is managing WiFi (read-only)
        let nm_status = Command::new("systemctl")
            .args(&["is-active", "NetworkManager", "--quiet"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        let wpa_status = Command::new("systemctl")
            .args(&["is-active", "wpa_supplicant", "--quiet"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        // Get driver information for first wireless interface
        let driver_info = if let Some(first_interface) = net_interfaces.first() {
            match fs::read_to_string(format!("/sys/class/net/{}/device/uevent", first_interface)) {
                Ok(content) => content
                    .lines()
                    .find(|line| line.starts_with("DRIVER="))
                    .map(|line| line.replace("DRIVER=", "")),
                Err(_) => None,
            }
        } else {
            None
        };

        let mut result_parts = vec![
            format!("Loaded kernel modules: {}", loaded_modules.join(", ")),
            format!("Wireless interfaces: {}", net_interfaces.join(", ")),
        ];

        if !interface_info.is_empty() {
            result_parts.push(format!("Interface details: {}", interface_info.join(", ")));
        }

        if let Some(driver) = driver_info {
            result_parts.push(format!("Driver: {}", driver));
        }

        let mut management_info = Vec::new();
        if nm_status {
            management_info.push("NetworkManager");
        }
        if wpa_status {
            management_info.push("wpa_supplicant");
        }

        if !management_info.is_empty() {
            result_parts.push(format!("Management: {}", management_info.join(", ")));
        }

        Ok(result_parts.join(" | "))
    }
}


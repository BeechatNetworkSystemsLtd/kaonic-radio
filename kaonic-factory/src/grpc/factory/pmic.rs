use std::fs;
use std::process::Command;
use super::FactoryTest;

pub struct PmicTest;

#[tonic::async_trait]
impl FactoryTest for PmicTest {
    fn name(&self) -> &str {
        "PMIC (Power Management IC) Test"
    }
    
    fn description(&self) -> &str {
        "Check PMIC functionality and power supply status"
    }
    
    async fn execute(&self) -> Result<String, String> {
        let mut pmic_info = Vec::new();
        let mut checks_performed = 0;
        let mut successful_checks = 0;
        
        // Method 1: Check I2C devices (PMICs are often on I2C bus)
        if let Ok(output) = Command::new("i2cdetect").args(&["-y", "-r", "0"]).output() {
            checks_performed += 1;
            if output.status.success() {
                let i2c_info = String::from_utf8_lossy(&output.stdout);
                let mut detected_addresses = Vec::new();
                
                for line in i2c_info.lines().skip(1) { // Skip header
                    for addr in line.split_whitespace().skip(1) { // Skip row address
                        if addr != "--" && addr.len() == 2 {
                            detected_addresses.push(format!("0x{}", addr));
                        }
                    }
                }
                
                if !detected_addresses.is_empty() {
                    successful_checks += 1;
                    pmic_info.push(format!("I2C devices detected: {}", detected_addresses.join(", ")));
                }
            }
        }
        
        // Method 2: Check /sys/class/power_supply for power supplies managed by PMIC
        if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
            checks_performed += 1;
            let mut power_supplies = Vec::new();
            
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.file_name().to_str() {
                    let supply_path = format!("/sys/class/power_supply/{}", name);
                    
                    // Read power supply type
                    let supply_type = fs::read_to_string(format!("{}/type", supply_path))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    
                    // Read online status
                    let online = fs::read_to_string(format!("{}/online", supply_path))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    
                    // Read voltage if available
                    let voltage = fs::read_to_string(format!("{}/voltage_now", supply_path))
                        .ok()
                        .and_then(|v| v.trim().parse::<i32>().ok())
                        .map(|v| v / 1000); // Convert µV to mV
                    
                    let mut supply_info = format!("{}: {} (online: {})", name, supply_type, online);
                    if let Some(v) = voltage {
                        supply_info.push_str(&format!(", {}mV", v));
                    }
                    
                    power_supplies.push(supply_info);
                }
            }
            
            if !power_supplies.is_empty() {
                successful_checks += 1;
                pmic_info.push(format!("Power supplies: {}", power_supplies.join("; ")));
            }
        }
        
        // Method 3: Check /sys/class/regulator for voltage regulators
        if let Ok(entries) = fs::read_dir("/sys/class/regulator") {
            checks_performed += 1;
            let mut regulators = Vec::new();
            
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.file_name().to_str() {
                    let reg_path = format!("/sys/class/regulator/{}", name);
                    
                    // Read regulator name
                    let reg_name = fs::read_to_string(format!("{}/name", reg_path))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    
                    // Read voltage
                    let voltage = fs::read_to_string(format!("{}/microvolts", reg_path))
                        .ok()
                        .and_then(|v| v.trim().parse::<i32>().ok())
                        .map(|v| v / 1000); // Convert µV to mV
                    
                    // Read state
                    let state = fs::read_to_string(format!("{}/state", reg_path))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    
                    let mut reg_info = format!("{} ({}): {}", name, reg_name, state);
                    if let Some(v) = voltage {
                        reg_info.push_str(&format!(", {}mV", v));
                    }
                    
                    regulators.push(reg_info);
                }
            }
            
            if !regulators.is_empty() {
                successful_checks += 1;
                pmic_info.push(format!("Regulators: {}", regulators.join("; ")));
            }
        }
        
        // Method 4: Check device tree for PMIC information
        let dt_pmic_paths = [
            "/proc/device-tree/soc/i2c*/pmic*",
            "/proc/device-tree/i2c*/pmic*",
            "/sys/firmware/devicetree/base/soc/i2c*/pmic*",
        ];
        
        for pattern in dt_pmic_paths.iter() {
            if let Ok(output) = Command::new("sh").args(&["-c", &format!("ls -d {} 2>/dev/null", pattern)]).output() {
                let paths = String::from_utf8_lossy(&output.stdout);
                for path in paths.lines() {
                    if !path.is_empty() {
                        checks_performed += 1;
                        if let Ok(compatible) = fs::read_to_string(format!("{}/compatible", path)) {
                            successful_checks += 1;
                            let compatible = compatible.trim().replace('\0', ", ");
                            pmic_info.push(format!("Device tree PMIC: {}", compatible));
                        }
                    }
                }
            }
        }
        
        // Method 5: Check common PMIC driver modules
        if let Ok(output) = Command::new("lsmod").output() {
            checks_performed += 1;
            let modules = String::from_utf8_lossy(&output.stdout);
            let pmic_modules = [
                "rk808", "rk818", "rk809", "rk817", // Rockchip PMICs
                "axp20x", "axp22x", // X-Powers PMICs
                "tps65910", "tps65912", "tps6586x", // TI PMICs
                "max77620", "max8997", "max8998", // Maxim PMICs
                "da9052", "da9055", "da9063", // Dialog PMICs
                "wm8994", "wm8350", // Wolfson PMICs
                "mc13xxx", "mc34708", // Freescale PMICs
            ];
            
            let mut loaded_pmic_modules = Vec::new();
            for module in pmic_modules.iter() {
                if modules.contains(module) {
                    loaded_pmic_modules.push(*module);
                }
            }
            
            if !loaded_pmic_modules.is_empty() {
                successful_checks += 1;
                pmic_info.push(format!("PMIC modules: {}", loaded_pmic_modules.join(", ")));
            }
        }
        
        // Method 6: Check for thermal zones (PMICs often have thermal monitoring)
        if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
            let mut pmic_thermal_zones = Vec::new();
            
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("thermal_zone") {
                        let zone_path = format!("/sys/class/thermal/{}", name);
                        if let Ok(zone_type) = fs::read_to_string(format!("{}/type", zone_path)) {
                            let zone_type = zone_type.trim();
                            if zone_type.contains("pmic") || zone_type.contains("PMIC") {
                                if let Ok(temp) = fs::read_to_string(format!("{}/temp", zone_path)) {
                                    if let Ok(temp_val) = temp.trim().parse::<i32>() {
                                        let temp_c = temp_val / 1000; // Convert milli-degrees to degrees
                                        pmic_thermal_zones.push(format!("{}: {}°C", zone_type, temp_c));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            if !pmic_thermal_zones.is_empty() {
                checks_performed += 1;
                successful_checks += 1;
                pmic_info.push(format!("PMIC thermal: {}", pmic_thermal_zones.join(", ")));
            }
        }
        
        if checks_performed == 0 {
            return Err("No PMIC check methods available on this system".to_string());
        }
        
        if successful_checks == 0 {
            return Err("No PMIC found - checked I2C devices, power supplies, regulators, device tree, and kernel modules".to_string());
        }
        
        let summary = format!("PMIC checks: {}/{} successful", successful_checks, checks_performed);
        
        if pmic_info.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{} | {}", summary, pmic_info.join(" | ")))
        }
    }
}
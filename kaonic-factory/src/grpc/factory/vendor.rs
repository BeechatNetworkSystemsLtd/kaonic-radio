use std::fs;
use super::FactoryTest;

pub struct VendorInfoTest;

#[tonic::async_trait]
impl FactoryTest for VendorInfoTest {
    fn name(&self) -> &str {
        "Vendor Information Test"
    }
    
    fn description(&self) -> &str {
        "Read and verify vendor information from system files"
    }
    
    async fn execute(&self) -> Result<String, String> {
        let mut vendor_info = Vec::new();
        
        // Common vendor files to check
        let vendor_files = [
            ("/etc/kaonic/kaonic_machine", "Kaonic Machine"),
            ("/etc/kaonic/kaonic_serial", "Kaonic Serial"),
        ];
        
        let mut found_files = 0;
        let mut valid_content = 0;
        
        for (file_path, description) in vendor_files.iter() {
            match fs::read_to_string(file_path) {
                Ok(content) => {
                    found_files += 1;
                    let content = content.trim();
                    
                    if !content.is_empty() && content != "To be filled by O.E.M." && content != "Not Specified" {
                        valid_content += 1;
                        vendor_info.push(format!("{}: {}", description, content));
                    } else {
                        vendor_info.push(format!("{}: <not specified>", description));
                    }
                }
                Err(_) => {
                    // File doesn't exist or can't be read, skip silently
                }
            }
        }
        
        // Check CPU info for additional vendor information
        if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
            found_files += 1;
            
            for line in cpuinfo.lines() {
                if line.starts_with("model name") || line.starts_with("Hardware") || line.starts_with("Model") {
                    if let Some(value) = line.split(':').nth(1) {
                        let value = value.trim();
                        if !value.is_empty() {
                            valid_content += 1;
                            let field = if line.starts_with("model name") {
                                "CPU Model"
                            } else if line.starts_with("Hardware") {
                                "Hardware"
                            } else {
                                "Model"
                            };
                            vendor_info.push(format!("{}: {}", field, value));
                            break; // Only take the first match
                        }
                    }
                }
            }
        }
        
        if found_files == 0 {
            return Err("No vendor information files found on system".to_string());
        }
        
        if valid_content == 0 {
            return Err("No valid vendor information found - all files empty or contain placeholder values".to_string());
        }
        
        let summary = format!("Found {} vendor files, {} with valid content", found_files, valid_content);
        
        if vendor_info.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{} | {}", summary, vendor_info.join(" | ")))
        }
    }
}

use std::fs;
use std::process::Command;
use super::FactoryTest;

pub struct MemoryTest;

#[tonic::async_trait]
impl FactoryTest for MemoryTest {
    fn name(&self) -> &str {
        "DDR Memory Test"
    }
    
    fn description(&self) -> &str {
        "Test DDR memory functionality, capacity, and performance"
    }
    
    async fn execute(&self) -> Result<String, String> {
        let mut memory_info = Vec::new();
        let mut checks_performed = 0;
        let mut successful_checks = 0;
        
        // Method 1: Check /proc/meminfo for memory information
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            checks_performed += 1;
            
            let mut total_memory = None;
            let mut free_memory = None;
            let mut available_memory = None;
            
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(value) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = value.parse::<u64>() {
                            total_memory = Some(kb / 1024); // Convert to MB
                        }
                    }
                } else if line.starts_with("MemFree:") {
                    if let Some(value) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = value.parse::<u64>() {
                            free_memory = Some(kb / 1024);
                        }
                    }
                } else if line.starts_with("MemAvailable:") {
                    if let Some(value) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = value.parse::<u64>() {
                            available_memory = Some(kb / 1024);
                        }
                    }
                }
            }
            
            if let Some(total) = total_memory {
                successful_checks += 1;
                let mut mem_status = format!("Total: {}MB", total);
                
                if let Some(free) = free_memory {
                    mem_status.push_str(&format!(", Free: {}MB", free));
                }
                
                if let Some(available) = available_memory {
                    mem_status.push_str(&format!(", Available: {}MB", available));
                    let usage_percent = ((total - available) * 100) / total;
                    mem_status.push_str(&format!(", Usage: {}%", usage_percent));
                }
                
                memory_info.push(format!("Memory info: {}", mem_status));
            }
        }
        
        // Method 2: Check DMI information for memory details
        let dmi_memory_paths = [
            "/sys/class/dmi/id/memory_array_maximum_capacity",
            "/sys/class/dmi/id/memory_array_number_devices",
        ];
        
        for (i, path) in dmi_memory_paths.iter().enumerate() {
            if let Ok(content) = fs::read_to_string(path) {
                checks_performed += 1;
                let content = content.trim();
                if !content.is_empty() && content != "Unknown" {
                    successful_checks += 1;
                    let field_name = if i == 0 { "Max capacity" } else { "Memory slots" };
                    memory_info.push(format!("DMI {}: {}", field_name, content));
                }
            }
        }
        
        // Method 3: Simple memory allocation test (safe, small allocations)
        checks_performed += 1;
        let allocation_result = self.test_memory_allocation().await;
        match allocation_result {
            Ok(result) => {
                successful_checks += 1;
                memory_info.push(format!("Allocation test: {}", result));
            }
            Err(e) => {
                memory_info.push(format!("Allocation test failed: {}", e));
            }
        }
        
        // Method 4: Check /proc/iomem for memory regions
        if let Ok(iomem) = fs::read_to_string("/proc/iomem") {
            checks_performed += 1;
            let mut memory_regions = Vec::new();
            
            for line in iomem.lines() {
                if line.contains("System RAM") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 {
                        let range = parts[0].trim();
                        if let Some((start, end)) = range.split_once('-') {
                            if let (Ok(start_addr), Ok(end_addr)) = 
                                (u64::from_str_radix(start.trim(), 16), u64::from_str_radix(end.trim(), 16)) {
                                let size_mb = (end_addr - start_addr + 1) / (1024 * 1024);
                                memory_regions.push(format!("0x{}-0x{} ({}MB)", start.trim(), end.trim(), size_mb));
                            }
                        }
                    }
                }
            }
            
            if !memory_regions.is_empty() {
                successful_checks += 1;
                memory_info.push(format!("RAM regions: {}", memory_regions.join(", ")));
            }
        }
        
        // Method 5: Check for memory errors in dmesg
        if let Ok(output) = Command::new("dmesg").output() {
            checks_performed += 1;
            let dmesg_content = String::from_utf8_lossy(&output.stdout);
            let memory_errors = [
                "memory error", "Memory error", "MEMORY ERROR",
                "ECC error", "ecc error",
                "Bad RAM", "bad ram",
                "Memory failure", "memory failure",
                "DIMM error", "dimm error",
            ];
            
            let mut error_count = 0;
            for error_pattern in memory_errors.iter() {
                error_count += dmesg_content.matches(error_pattern).count();
            }
            
            if error_count == 0 {
                successful_checks += 1;
                memory_info.push("Memory error check: No errors found in dmesg".to_string());
            } else {
                memory_info.push(format!("Memory error check: {} potential errors found in dmesg", error_count));
            }
        }
        
        // Method 6: Read memory speed from DMI if available
        if let Ok(entries) = fs::read_dir("/sys/devices/virtual/dmi/id") {
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("memory_device") && name.contains("speed") {
                        if let Ok(speed) = fs::read_to_string(entry.path()) {
                            let speed = speed.trim();
                            if !speed.is_empty() && speed != "Unknown" && speed != "0" {
                                checks_performed += 1;
                                successful_checks += 1;
                                memory_info.push(format!("Memory speed: {}", speed));
                                break; // Only report first valid speed found
                            }
                        }
                    }
                }
            }
        }
        
        // Method 7: Check memory bandwidth with a simple test
        checks_performed += 1;
        let bandwidth_result = self.test_memory_bandwidth().await;
        match bandwidth_result {
            Ok(result) => {
                successful_checks += 1;
                memory_info.push(format!("Bandwidth test: {}", result));
            }
            Err(e) => {
                memory_info.push(format!("Bandwidth test: {}", e));
            }
        }
        
        if checks_performed == 0 {
            return Err("No memory check methods available".to_string());
        }
        
        if successful_checks == 0 {
            return Err("All memory checks failed - potential memory hardware issue".to_string());
        }
        
        let summary = format!("Memory checks: {}/{} successful", successful_checks, checks_performed);
        
        if memory_info.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{} | {}", summary, memory_info.join(" | ")))
        }
    }
}

impl MemoryTest {
    async fn test_memory_allocation(&self) -> Result<String, String> {
        // Safe memory allocation test - allocate small chunks and verify
        const TEST_SIZE_MB: usize = 10; // Only test with 10MB to be safe
        const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
        
        let mut allocated_chunks = Vec::new();
        let test_pattern = 0x5A; // Test pattern
        
        for i in 0..TEST_SIZE_MB {
            let mut chunk = vec![0u8; CHUNK_SIZE];
            
            // Fill with test pattern
            for byte in &mut chunk {
                *byte = test_pattern;
            }
            
            // Verify pattern
            for (j, &byte) in chunk.iter().enumerate() {
                if byte != test_pattern {
                    return Err(format!("Memory verification failed at chunk {}, byte {}", i, j));
                }
            }
            
            allocated_chunks.push(chunk);
        }
        
        // Clear and verify clearing
        for (i, chunk) in allocated_chunks.iter_mut().enumerate() {
            chunk.fill(0);
            for (j, &byte) in chunk.iter().enumerate() {
                if byte != 0 {
                    return Err(format!("Memory clear failed at chunk {}, byte {}", i, j));
                }
            }
        }
        
        Ok(format!("{}MB allocation/verification passed", TEST_SIZE_MB))
    }
    
    async fn test_memory_bandwidth(&self) -> Result<String, String> {
        use std::time::Instant;
        
        const TEST_SIZE: usize = 1024 * 1024; // 1MB test
        let data = vec![0u8; TEST_SIZE];
        let mut dest = vec![0u8; TEST_SIZE];
        
        let start = Instant::now();
        
        // Perform memory copy operations to test bandwidth
        for _ in 0..100 {
            dest.copy_from_slice(&data);
        }
        
        let duration = start.elapsed();
        let bytes_copied = (TEST_SIZE * 100) as f64;
        let bandwidth_mb_s = (bytes_copied / (1024.0 * 1024.0)) / duration.as_secs_f64();
        
        Ok(format!("{:.1} MB/s copy bandwidth", bandwidth_mb_s))
    }
}
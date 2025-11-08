use kaonic_radio::{
    error::KaonicError,
    platform::{create_machine, kaonic1s::Kaonic1SRadio},
};

use super::FactoryTest;
use std::process::Command;

pub struct Rf215Test;

#[tonic::async_trait]
impl FactoryTest for Rf215Test {
    fn name(&self) -> &str {
        "RF215 Transceivers Test"
    }

    fn description(&self) -> &str {
        "Test RF215 transceivers on SPI 3.0 and 6.0 using existing driver"
    }

    async fn execute(&self) -> Result<String, String> {
        // Stop kaonic-commd service before testing
        self.stop_kaonic_service().await?;

        // Perform the RF215 tests
        let test_result = self.perform_rf215_tests().await;

        // Always restart the service, even if tests failed
        let restart_result = self.start_kaonic_service().await;

        // Handle the results
        match test_result {
            Ok(info) => {
                // Check if service restart failed
                if let Err(restart_error) = restart_result {
                    return Err(format!(
                        "RF215 tests passed but failed to restart kaonic-commd: {}",
                        restart_error
                    ));
                }
                Ok(info)
            }
            Err(test_error) => {
                // Include restart error if it occurred
                if let Err(restart_error) = restart_result {
                    Err(format!(
                        "{} | Failed to restart kaonic-commd: {}",
                        test_error, restart_error
                    ))
                } else {
                    Err(test_error)
                }
            }
        }
    }
}

impl Rf215Test {
    async fn stop_kaonic_service(&self) -> Result<(), String> {
        let stop_output = Command::new("systemctl")
            .args(&["stop", "kaonic-commd.service"])
            .output()
            .map_err(|e| format!("Failed to stop kaonic-commd service: {}", e))?;

        if !stop_output.status.success() {
            let error_msg = String::from_utf8_lossy(&stop_output.stderr);
            return Err(format!(
                "Failed to stop kaonic-commd service: {}",
                error_msg
            ));
        }

        // Wait for service to fully stop
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        Ok(())
    }

    async fn start_kaonic_service(&self) -> Result<(), String> {
        let start_output = Command::new("systemctl")
            .args(&["start", "kaonic-commd.service"])
            .output()
            .map_err(|e| format!("Failed to start kaonic-commd service: {}", e))?;

        if !start_output.status.success() {
            let error_msg = String::from_utf8_lossy(&start_output.stderr);
            return Err(format!(
                "Failed to start kaonic-commd service: {}",
                error_msg
            ));
        }

        // Wait for service to start up
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        Ok(())
    }

    async fn perform_rf215_tests(&self) -> Result<String, String> {
        let mut machine = create_machine().map_err(|_| format!("Failed to create machine"))?;

        let radio_names = ["RF215-A", "RF215-B"];

        let results = machine
            .for_each_radio(|idx, radio| {
                match radio {
                    Some(r) => self
                        .test_rf215_instance(r, radio_names[idx])
                        .map(|info| format!("{}: {}", radio_names[idx], info))
                        .map_err(|_| KaonicError::IncorrectSettings),
                    None => Ok(format!(
                        "{}: not connected (hardware missing or configuration error)",
                        radio_names[idx]
                    )),
                }
            })
            .map_err(|e| format!("RF215 iteration error: {:?}", e))?;

        if results.is_empty() {
            return Err("No RF215 radios were successfully initialized".to_string());
        }

        Ok(results.join(" | "))
    }

    fn test_rf215_instance(
        &self,
        radio: &mut Kaonic1SRadio,
        _radio_name: &str,
    ) -> Result<String, String> {
        let radio = radio.radio();
        // Get radio information using the existing driver methods
        let part_number = radio.part_number();
        let version_number = radio.version();

        // Validate version number
        if version_number < 0x01 {
            return Err(format!(
                "Invalid version: expected >= 0x01, got 0x{:02X}",
                version_number
            ));
        }

        Ok(format!("PN=0x{:02X}, VN=0x{:02X}", part_number as u8, version_number))
    }
}

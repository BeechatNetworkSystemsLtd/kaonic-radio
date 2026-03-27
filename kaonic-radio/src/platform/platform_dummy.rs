use std::sync::{Arc, Mutex};

use kaonic_frame::frame::Frame;
use radio_common::{modulation::OfdmModulation, Modulation, RadioConfig, RadioConfigBuilder};

use crate::{
    error::KaonicError,
    radio::{Radio, ReceiveResult, ScanResult},
};

pub type DummyFrame = Frame<2048>;

pub struct DummyRadioEvent;

impl DummyRadioEvent {
    pub fn wait_for_event(&mut self, timeout: Option<core::time::Duration>) -> bool {
        // No hardware events on the host platform; sleep briefly so the event
        // thread stays responsive to shutdown without busy-looping.
        let sleep = timeout.unwrap_or(core::time::Duration::from_millis(100));
        std::thread::sleep(sleep);
        false
    }
}

pub struct DummyRadio {
    event: Arc<Mutex<DummyRadioEvent>>,
}

impl DummyRadio {
    pub fn new() -> Self {
        Self {
            event: Arc::new(Mutex::new(DummyRadioEvent)),
        }
    }

    pub fn event(&self) -> Arc<Mutex<DummyRadioEvent>> {
        self.event.clone()
    }
}

pub struct DummyMachine {
    radio_count: usize,
}

impl DummyMachine {
    pub fn new() -> Result<Self, KaonicError> {
        Ok(DummyMachine { radio_count: 2 })
    }

    pub fn take_radio(&mut self, index: usize) -> Option<DummyRadio> {
        if index < self.radio_count {
            Some(DummyRadio::new())
        } else {
            None
        }
    }
}

impl Radio for DummyRadio {
    type TxFrame = DummyFrame;
    type RxFrame = DummyFrame;

    fn update_event(&mut self) -> Result<(), KaonicError> {
        Ok(())
    }

    fn set_config(&mut self, _config: &RadioConfig) -> Result<(), KaonicError> {
        Ok(())
    }

    fn get_config(&self) -> RadioConfig {
        RadioConfigBuilder::new().build()
    }

    fn set_modulation(&mut self, _modulation: &Modulation) -> Result<(), KaonicError> {
        Ok(())
    }

    fn get_modulation(&self) -> Modulation {
        Modulation::Ofdm(OfdmModulation::default())
    }

    fn transmit(&mut self, _frame: &Self::TxFrame) -> Result<(), KaonicError> {
        Err(KaonicError::HardwareError)
    }

    fn receive<'a>(
        &mut self,
        _frame: &'a mut Self::RxFrame,
        _timeout: core::time::Duration,
    ) -> Result<ReceiveResult, KaonicError> {
        Err(KaonicError::HardwareError)
    }

    fn scan(&mut self, _timeout: core::time::Duration) -> Result<ScanResult, KaonicError> {
        Err(KaonicError::HardwareError)
    }
}

pub fn create_machine() -> Result<DummyMachine, KaonicError> {
    DummyMachine::new()
}

pub type PlatformRadio = DummyRadio;
pub type PlatformRadioEvent = DummyRadioEvent;
pub type PlatformRadioFrame = DummyFrame;

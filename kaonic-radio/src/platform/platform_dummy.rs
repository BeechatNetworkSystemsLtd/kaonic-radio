use crate::{
    error::KaonicError,
    modulation::Modulation,
    radio::{Radio, RadioConfig, ReceiveResult, ScanResult},
};

pub struct DummyMachine;
pub struct DummyRadio;

// Frame type compatible with kaonic1s
pub type Frame<const N: usize> = [u8; N];
pub type DummyFrame = Frame<2048>;

impl DummyMachine {
    pub fn new() -> Result<Self, KaonicError> {
        Ok(DummyMachine)
    }

    pub fn take_radio(&mut self, _index: usize) -> Option<DummyRadio> {
        Some(DummyRadio)
    }

    pub fn for_each_radio<T, F>(&mut self, _f: F) -> Result<[T; 2], KaonicError>
    where
        F: FnMut(usize, &mut Option<DummyRadio>) -> Result<T, KaonicError>,
        T: Clone,
    {
        Err(KaonicError::HardwareError)
    }
}

impl Radio for DummyRadio {
    type TxFrame = DummyFrame;
    type RxFrame = DummyFrame;

    fn update_event(&mut self) -> Result<(), KaonicError> {
        Ok(())
    }

    fn configure(&mut self, _config: &RadioConfig) -> Result<(), KaonicError> {
        Ok(())
    }

    fn set_modulation(&mut self, _modulation: &Modulation) -> Result<(), KaonicError> {
        Ok(())
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

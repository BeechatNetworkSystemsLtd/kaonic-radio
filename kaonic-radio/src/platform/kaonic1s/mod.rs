//! The Kaonic 1S board.
//!
//! The board is the same whichever way its radios are driven; only the driver
//! differs. `us` runs the RF215 driver in user space over spidev, `ks` talks
//! to the in-kernel `kaonic_rf215` driver through its network interfaces.
//! Both present the same `Kaonic1SRadio`, so everything above this module is
//! shared.

use std::sync::{Arc, Mutex};

use kaonic_frame::frame::Frame;

use crate::{
    error::KaonicError,
    platform::kaonic1s::{config::fpga_enable_gpio, fpga::Kaonic1SFpga},
};

mod config;
pub mod fem;
pub mod fpga;

#[cfg(feature = "machine-kaonic1s-us")]
#[path = "us/mod.rs"]
mod driver;

#[cfg(feature = "machine-kaonic1s-ks")]
#[path = "ks/mod.rs"]
mod driver;

pub use driver::*;
pub use fem::Kaonic1SRadioFem;

pub const FRAME_SIZE: usize = 2048usize;

pub type Kaonic1SFrame = Frame<FRAME_SIZE>;

pub const KAONIC1S_RADIO_COUNT: usize = 2;

pub struct Kaonic1SMachine {
    radios: [Option<Kaonic1SRadio>; KAONIC1S_RADIO_COUNT],
    fpga: Option<Arc<Mutex<Kaonic1SFpga>>>,
}

impl Kaonic1SMachine {
    pub fn new() -> Result<Self, KaonicError> {
        let fpga = Kaonic1SFpga::new(fpga_enable_gpio())
            .map(|fpga| Arc::new(Mutex::new(fpga)))
            .ok();

        if fpga.is_none() {
            log::warn!("fpga module wasn't found");
        } else {
            log::info!("fpga module found");
        }

        let radios = driver::create_radios(fpga.clone()).map_err(|_| KaonicError::HardwareError)?;

        Ok(Self { radios, fpga })
    }

    pub fn fpga(&self) -> Option<Arc<Mutex<Kaonic1SFpga>>> {
        self.fpga.clone()
    }

    pub fn take_radio(&mut self, index: usize) -> Option<Kaonic1SRadio> {
        if index < self.radios.len() {
            self.radios[index].take()
        } else {
            None
        }
    }

    pub fn for_each_radio<T, F>(
        &mut self,
        mut f: F,
    ) -> Result<[T; KAONIC1S_RADIO_COUNT], KaonicError>
    where
        F: FnMut(usize, &mut Option<Kaonic1SRadio>) -> Result<T, KaonicError>,
        T: Clone,
    {
        let mut results: [Result<T, KaonicError>; KAONIC1S_RADIO_COUNT] = [
            Err(KaonicError::HardwareError),
            Err(KaonicError::HardwareError),
        ];

        for (index, radio) in self.radios.iter_mut().enumerate() {
            results[index] = f(index, radio);
        }

        for r in results.iter() {
            if r.is_err() {
                return Err(KaonicError::IncorrectSettings);
            }
        }

        Ok(results.map(|r| r.unwrap()))
    }
}

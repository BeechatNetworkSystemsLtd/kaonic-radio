use kaonic_radio::{error::KaonicError, platform::linux::LinuxOutputPin};
use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;

pub struct Kaonic1SFpga {
    enable_gpio: LinuxOutputPin,
    mmap: MmapMut,
}

const PSRAM_SIZE: usize = 1;
const PSRAM_OFFSET: u64 = 0x64000000;

impl Kaonic1SFpga {
    pub fn new() -> Result<Self, KaonicError> {
        let mut enable_gpio = LinuxOutputPin::new_from_line("/dev/gpiochip9", 7, "kaonic-fpga-en")?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/mem")
            .map_err(|_| KaonicError::IncorrectSettings)?;

        let mmap = unsafe {
            MmapOptions::new()
                .offset(PSRAM_OFFSET as u64)
                .len(PSRAM_SIZE)
                .map_mut(&file)
                .map_err(|_| KaonicError::HardwareError)?
        };

        enable_gpio.set_low()?;

        Ok(Self { enable_gpio, mmap })
    }

    pub fn write_byte(&mut self, byte: u8) -> Result<(), KaonicError> {
        let addr = self.mmap.as_mut_ptr();

        unsafe { core::ptr::write_volatile(addr as *mut u8, byte as u8) }

        Ok(())
    }

    pub fn read_byte(&mut self) -> Result<u8, KaonicError> {
        let addr = self.mmap.as_mut_ptr();

        let byte = unsafe { core::ptr::read_volatile(addr as *const u8) as u8 };

        Ok(byte)
    }

    pub fn enable(&mut self) -> Result<(), KaonicError> {
        self.enable_gpio.set_high()
    }

    pub fn disable(&mut self) -> Result<(), KaonicError> {
        self.enable_gpio.set_low()
    }
}

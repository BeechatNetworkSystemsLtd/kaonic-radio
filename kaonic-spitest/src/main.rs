//! RF215 SPI bus integrity tester: write/readback patterns over registers
//! and the frame buffer at increasing SPI clock speeds, reporting error
//! rates and effective bus throughput per speed.

use clap::Parser;
use embedded_hal::spi::{Operation, SpiDevice};
use linux_embedded_hal::spidev::{SpiModeFlags, SpidevOptions};
use linux_embedded_hal::SpidevDevice;
use std::time::Instant;

const RG_RF_PN: u16 = 0x0D;
const RG_RF_VN: u16 = 0x0E;
// BBC0 MAC address registers: plain read/write storage, safe scratch space.
const RG_BBC0_MACEA0: u16 = 0x0325;
// BBC0 TX frame buffer (0x2800..0x2FFF).
const RG_BBC0_FBTXS: u16 = 0x2800;

const WRITE: u16 = 0x8000;

#[derive(Parser)]
struct Args {
    /// SPI device path
    #[arg(long, default_value = "/dev/spidev6.0")]
    dev: String,

    /// Speeds to test, in Hz
    #[arg(long, value_delimiter = ',', default_values_t = vec![12_000_000u32, 16_000_000, 20_000_000, 25_000_000])]
    speeds: Vec<u32>,

    /// Register-pattern iterations per speed
    #[arg(long, default_value_t = 2000)]
    reg_iters: u32,

    /// Frame-buffer-pattern iterations per speed
    #[arg(long, default_value_t = 200)]
    fb_iters: u32,
}

fn write_regs(spi: &mut SpidevDevice, addr: u16, data: &[u8]) -> bool {
    let addr = (addr | WRITE).to_be_bytes();
    spi.transaction(&mut [Operation::Write(&addr), Operation::Write(data)])
        .is_ok()
}

fn read_regs(spi: &mut SpidevDevice, addr: u16, data: &mut [u8]) -> bool {
    let addr = addr.to_be_bytes();
    spi.transaction(&mut [Operation::Write(&addr), Operation::Read(data)])
        .is_ok()
}

fn main() {
    let args = Args::parse();

    let mut spi = SpidevDevice::open(&args.dev).expect("open spidev");

    for &speed in &args.speeds {
        spi.configure(
            &SpidevOptions::new()
                .max_speed_hz(speed)
                .mode(SpiModeFlags::SPI_MODE_0)
                .build(),
        )
        .expect("configure spi");

        // Sanity: part number must read back as an RF215 family id.
        let mut pn = [0u8; 2];
        read_regs(&mut spi, RG_RF_PN, &mut pn);
        let _ = RG_RF_VN;

        let mut reg_errors = 0u64;
        let mut reg_io_fail = 0u64;

        for i in 0..args.reg_iters {
            let mut pattern = [0u8; 8];
            for (k, b) in pattern.iter_mut().enumerate() {
                *b = ((i as usize).wrapping_mul(31).wrapping_add(k * 97) & 0xFF) as u8;
            }

            if !write_regs(&mut spi, RG_BBC0_MACEA0, &pattern) {
                reg_io_fail += 1;
                continue;
            }

            let mut readback = [0u8; 8];
            if !read_regs(&mut spi, RG_BBC0_MACEA0, &mut readback) {
                reg_io_fail += 1;
                continue;
            }

            reg_errors += pattern
                .iter()
                .zip(readback.iter())
                .filter(|(a, b)| a != b)
                .count() as u64;
        }

        let mut fb_errors = 0u64;
        let mut fb_io_fail = 0u64;
        let mut fb_bytes = 0u64;

        let start = Instant::now();

        for i in 0..args.fb_iters {
            let mut pattern = [0u8; 2048];
            for (k, b) in pattern.iter_mut().enumerate() {
                *b = ((k as u32)
                    .wrapping_mul(197)
                    .wrapping_add(i.wrapping_mul(13)) & 0xFF) as u8;
            }

            if !write_regs(&mut spi, RG_BBC0_FBTXS, &pattern) {
                fb_io_fail += 1;
                continue;
            }

            let mut readback = [0u8; 2048];
            if !read_regs(&mut spi, RG_BBC0_FBTXS, &mut readback) {
                fb_io_fail += 1;
                continue;
            }

            fb_bytes += 2 * 2048;

            fb_errors += pattern
                .iter()
                .zip(readback.iter())
                .filter(|(a, b)| a != b)
                .count() as u64;
        }

        let elapsed = start.elapsed();
        let mbps = fb_bytes as f64 * 8.0 / elapsed.as_secs_f64() / 1e6;

        println!(
            "speed {:>2} MHz | PN {:02x}{:02x} | reg: {} byte-err {} io-fail / {} iters | fb: {} byte-err {} io-fail / {} iters | eff bus {:>5.1} Mbit/s",
            speed / 1_000_000,
            pn[0],
            pn[1],
            reg_errors,
            reg_io_fail,
            args.reg_iters,
            fb_errors,
            fb_io_fail,
            args.fb_iters,
            mbps,
        );
    }
}

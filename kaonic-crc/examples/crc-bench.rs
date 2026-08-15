//! Measures the software and hardware CRC backends on the current machine so
//! the hardware threshold can be set from data instead of a guess.
//!
//! The "crossover" line is the smallest measured size where hardware beats
//! software; feed it back as the `MIN_HW_LEN` constant in `src/af_alg.rs`.

use std::time::{Duration, Instant};

/// Payload sizes spanning a radio frame (tens of bytes) up to bulk transfers.
const SIZES: &[usize] = &[
    16, 64, 128, 256, 512, 1024, 1500, 2048, 4096, 16 * 1024, 64 * 1024, 1024 * 1024,
];

/// Minimum wall time per measurement; iteration count adapts to reach it.
const MIN_SAMPLE: Duration = Duration::from_millis(50);

fn pseudo_random(len: usize, mut seed: u32) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect()
}

/// Runs `f` until `MIN_SAMPLE` elapses; returns nanoseconds per call.
fn measure(data: &[u8], mut f: impl FnMut(&[u8]) -> u32) -> f64 {
    // Warm up: first call may open a socket or fault in pages.
    let mut checksum = f(data);

    let mut iterations = 0u32;
    let start = Instant::now();
    while start.elapsed() < MIN_SAMPLE {
        for _ in 0..16 {
            checksum ^= f(data);
        }
        iterations += 16;
    }
    let elapsed = start.elapsed();

    // Keep the optimiser honest about the checksum being used.
    std::hint::black_box(checksum);

    elapsed.as_nanos() as f64 / iterations as f64
}

fn throughput_mb_s(len: usize, nanos: f64) -> f64 {
    (len as f64 / (nanos / 1e9)) / (1024.0 * 1024.0)
}

fn main() {
    #[cfg(feature = "machine-kaonic1s")]
    let hardware = kaonic_crc::hw_available();
    #[cfg(not(feature = "machine-kaonic1s"))]
    let hardware = false;

    println!("backend in use : {:?}", kaonic_crc::backend());
    println!("hw available   : {}", hardware);
    if hardware {
        println!("hw threshold   : {} bytes", kaonic_crc::hw_threshold());
    } else {
        println!("hw threshold   : n/a");
    }
    println!();

    if !hardware {
        println!("hardware backend unavailable — measuring software only");
    }

    println!(
        "{:>10}  {:>12} {:>10}  {:>12} {:>10}  {:>8}",
        "size", "sw ns/op", "sw MB/s", "hw ns/op", "hw MB/s", "speedup"
    );

    let mut crossover: Option<usize> = None;

    for &size in SIZES {
        let data = pseudo_random(size, 0xC0FF_EE00 + size as u32);

        let sw = measure(&data, kaonic_crc::sw_crc32);

        #[cfg(feature = "machine-kaonic1s")]
        let hw = hardware.then(|| {
            measure(&data, |d| {
                kaonic_crc::hw_crc32(d).expect("hardware checksum failed mid-benchmark")
            })
        });
        #[cfg(not(feature = "machine-kaonic1s"))]
        let hw: Option<f64> = None;

        match hw {
            Some(hw) => {
                let speedup = sw / hw;
                if speedup > 1.0 && crossover.is_none() {
                    crossover = Some(size);
                }
                println!(
                    "{:>10}  {:>12.0} {:>10.1}  {:>12.0} {:>10.1}  {:>7.2}x",
                    size,
                    sw,
                    throughput_mb_s(size, sw),
                    hw,
                    throughput_mb_s(size, hw),
                    speedup
                );
            }
            None => println!(
                "{:>10}  {:>12.0} {:>10.1}  {:>12} {:>10}  {:>8}",
                size,
                sw,
                throughput_mb_s(size, sw),
                "-",
                "-",
                "-"
            ),
        }
    }

    println!();
    match crossover {
        Some(size) => println!(
            "crossover: hardware wins from {} bytes up — consider MIN_HW_LEN = {}",
            size, size
        ),
        None if hardware => {
            println!("crossover: none — software is faster at every measured size")
        }
        None => {}
    }
}

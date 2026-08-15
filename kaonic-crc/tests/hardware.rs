//! On-target hardware validation, kaonic1s builds only. Run manually on the
//! board with: cargo test -p kaonic-crc -- --ignored --test-threads=1
//!
//! `--test-threads=1` matters: the routing tests read a process-wide counter of
//! hardware operations, which another test hashing concurrently would disturb.
#![cfg(feature = "machine-kaonic1s")]

use kaonic_crc::{crc32, hw_available, hw_crc32, sw_crc32, Backend, Crc32Hasher};

/// Deterministic pseudo-random bytes without pulling in a rand dependency.
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

#[test]
#[ignore]
fn hardware_backend_is_active() {
    assert!(
        hw_available(),
        "AF_ALG crc32 backend did not pass its probe on this kernel"
    );
    assert_eq!(kaonic_crc::backend(), Backend::Hardware);
}

#[test]
#[ignore]
fn hardware_matches_software_on_vectors() {
    assert!(hw_available());
    for &(data, expected) in &[
        (b"" as &[u8], 0x0000_0000u32),
        (b"123456789", 0xCBF4_3926),
        (b"The quick brown fox jumps over the lazy dog", 0x414F_A339),
    ] {
        assert_eq!(crc32(data), expected);
        assert_eq!(crc32(data), crc32fast::hash(data));
        // Bypass the size threshold so the hardware path is exercised even
        // for inputs the public API would route to software.
        assert_eq!(hw_crc32(data), Some(expected), "hardware crc32 of {:?}", data);
    }
}

#[test]
#[ignore]
fn hardware_matches_software_on_random_buffers() {
    assert!(hw_available());
    for (index, &size) in [0usize, 1, 3, 64, 1500, 2048, 65 * 1024, 1024 * 1024]
        .iter()
        .enumerate()
    {
        let data = pseudo_random(size, 0xDEAD_0001 + index as u32);
        assert_eq!(
            crc32(&data),
            crc32fast::hash(&data),
            "mismatch at size {}",
            size
        );
        assert_eq!(
            hw_crc32(&data),
            Some(sw_crc32(&data)),
            "hardware mismatch at size {}",
            size
        );
    }
}

/// Data is well above the threshold so the hardware hasher is engaged, and
/// the chunk sizes cover every coalescing path: chunks that fit the 64 KiB
/// buffer, chunks that force a flush mid-buffer, and chunks bigger than the
/// buffer itself (head/tail split).
#[test]
#[ignore]
fn streaming_matches_oneshot_on_hardware() {
    assert!(hw_available());
    let data = pseudo_random(200_000, 0xBEEF_CAFE);
    let expected = crc32fast::hash(&data);

    for chunk_size in [1usize, 7, 4096, 64 * 1024 - 1, 64 * 1024, 100_000, 200_000] {
        let before = kaonic_crc::hw_ops();
        let mut hasher = Crc32Hasher::new();
        for chunk in data.chunks(chunk_size) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finalize(), expected, "chunk size {}", chunk_size);
        assert!(
            kaonic_crc::hw_ops() > before,
            "chunk size {} did not reach the hardware",
            chunk_size
        );
    }
}

/// Streams shorter than the hardware threshold never open a socket; make sure
/// that buffered path still produces the right answer on the target.
#[test]
#[ignore]
fn short_streams_stay_correct() {
    assert!(hw_available());
    for len in [0usize, 1, 15, 64, 255] {
        let data = pseudo_random(len, 0x5EED_0000 + len as u32);
        let mut hasher = Crc32Hasher::new();
        for chunk in data.chunks(4) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finalize(), sw_crc32(&data), "short stream len {}", len);
    }
}

/// The routing rule: payloads of 16 KiB and up go to the peripheral when it is
/// available, everything smaller stays in software. Verified by watching the
/// hardware operation counter rather than by trusting the constant.
#[test]
#[ignore]
fn threshold_routes_by_payload_size() {
    assert!(hw_available());
    assert_eq!(
        kaonic_crc::hw_threshold(),
        16 * 1024,
        "default hardware threshold changed"
    );

    // Warm up so socket setup is not counted, then measure per case.
    let _ = crc32(&pseudo_random(64 * 1024, 1));

    for (len, expect_hw) in [
        (1usize, false),
        (2048, false),           // radio frame: software
        (16 * 1024 - 1, false),  // just below the threshold
        (16 * 1024, true),       // exactly at it: hardware
        (64 * 1024, true),
    ] {
        let data = pseudo_random(len, 0xA5A5_0000 + len as u32);

        let before = kaonic_crc::hw_ops();
        let value = crc32(&data);
        let used_hw = kaonic_crc::hw_ops() > before;

        assert_eq!(value, sw_crc32(&data), "wrong checksum at len {}", len);
        assert_eq!(
            used_hw, expect_hw,
            "len {} took the {} path, expected {}",
            len,
            if used_hw { "hardware" } else { "software" },
            if expect_hw { "hardware" } else { "software" }
        );
    }
}

/// Same rule for the streaming hasher, which cannot know the total length up
/// front: it buffers until the stream crosses the threshold.
#[test]
#[ignore]
fn streaming_threshold_routes_by_total_size() {
    assert!(hw_available());

    for (len, expect_hw) in [(2048usize, false), (64 * 1024, true)] {
        let data = pseudo_random(len, 0x1357_0000 + len as u32);

        let before = kaonic_crc::hw_ops();
        let mut hasher = Crc32Hasher::new();
        for chunk in data.chunks(512) {
            hasher.update(chunk);
        }
        let value = hasher.finalize();
        let used_hw = kaonic_crc::hw_ops() > before;

        assert_eq!(value, sw_crc32(&data), "wrong checksum for stream of {}", len);
        assert_eq!(used_hw, expect_hw, "stream of {} took the wrong path", len);
    }
}

#[test]
#[ignore]
fn multithreaded_hammer() {
    assert!(hw_available());
    let handles: Vec<_> = (0..8u32)
        .map(|thread| {
            std::thread::spawn(move || {
                for round in 0..10_000u32 {
                    let data = pseudo_random(64 + (round as usize % 512), thread * 31 + round);
                    assert_eq!(crc32(&data), crc32fast::hash(&data));
                    assert_eq!(hw_crc32(&data), Some(sw_crc32(&data)));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("hammer thread panicked");
    }
}

//! CRC-32/ISO-HDLC (zlib-compatible) with optional hardware acceleration.
//!
//! On `machine-kaonic1s` builds the checksum is computed by the STM32MP1
//! CRC1 peripheral through the Linux kernel crypto API (AF_ALG). If the
//! kernel does not provide a working `crc32` hash, the crate falls back to
//! software transparently. On `machine-host` builds only the software
//! implementation is compiled.
//!
//! Every hardware checksum costs at least two syscalls, so inputs shorter
//! than [`hw_threshold`] are computed in software regardless of what the
//! kernel offers.

mod software;

#[cfg(feature = "machine-kaonic1s")]
mod af_alg;

/// Which implementation the crate is serving checksums from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `crc32fast`, always available.
    Software,
    /// Kernel `crc32` hash via AF_ALG, backed by the STM32 CRC1 peripheral.
    Hardware,
}

/// One-shot CRC-32/ISO-HDLC over `data`.
#[inline]
pub fn crc32(data: &[u8]) -> u32 {
    // `af_alg::crc32` applies the size threshold itself, so the hot path is a
    // single load plus one length comparison before the software fallback.
    #[cfg(feature = "machine-kaonic1s")]
    if let Some(value) = af_alg::crc32(data) {
        return value;
    }

    software::crc32(data)
}

/// Probes the backend and pre-opens this thread's socket, so the first
/// checksum on the hot path does not pay for it. Optional — everything
/// initializes lazily otherwise. Call once per worker thread at startup.
pub fn init() {
    #[cfg(feature = "machine-kaonic1s")]
    af_alg::warm_up();
}

/// Returns true when the hardware (AF_ALG) backend passed its probe and is
/// serving checksums. Always false on `machine-host` builds.
pub fn hw_available() -> bool {
    #[cfg(feature = "machine-kaonic1s")]
    {
        af_alg::available()
    }

    #[cfg(not(feature = "machine-kaonic1s"))]
    {
        false
    }
}

/// Backend that [`crc32`] uses for inputs at or above [`hw_threshold`].
pub fn backend() -> Backend {
    if hw_available() {
        Backend::Hardware
    } else {
        Backend::Software
    }
}

/// Inputs shorter than this always use software, even when hardware is
/// available. `usize::MAX` when there is no hardware backend.
pub fn hw_threshold() -> usize {
    #[cfg(feature = "machine-kaonic1s")]
    {
        af_alg::min_hw_len()
    }

    #[cfg(not(feature = "machine-kaonic1s"))]
    {
        usize::MAX
    }
}

/// Number of checksums the hardware backend has served since process start.
#[cfg(feature = "machine-kaonic1s")]
pub fn hw_ops() -> u64 {
    af_alg::hw_ops()
}

/// Hardware checksum, bypassing the size threshold. `None` when the hardware
/// backend is unavailable or the operation failed. Normal callers want
/// [`crc32`], which routes by size and falls back automatically.
#[cfg(feature = "machine-kaonic1s")]
pub fn hw_crc32(data: &[u8]) -> Option<u32> {
    if data.is_empty() {
        // The empty-message CRC needs no syscalls, but the backend still has
        // to be up for the answer to mean "hardware works".
        return hw_available().then(|| software::crc32(&[]));
    }

    af_alg::crc32_unchecked(data)
}

/// Software checksum, bypassing hardware entirely.
pub fn sw_crc32(data: &[u8]) -> u32 {
    software::crc32(data)
}

/// Streaming hasher; produces the same result as [`crc32`] over the
/// concatenation of all `update` inputs.
pub struct Crc32Hasher {
    inner: HasherImpl,
}

enum HasherImpl {
    Software(crc32fast::Hasher),
    /// Hardware is available, but the stream is still shorter than the
    /// threshold: bytes are held back until it is worth opening a socket.
    #[cfg(feature = "machine-kaonic1s")]
    Pending(Vec<u8>),
    #[cfg(feature = "machine-kaonic1s")]
    Hardware(af_alg::HwHasher),
}

impl Crc32Hasher {
    pub fn new() -> Self {
        #[cfg(feature = "machine-kaonic1s")]
        if af_alg::available() {
            return Self {
                inner: HasherImpl::Pending(Vec::new()),
            };
        }

        Self {
            inner: HasherImpl::Software(crc32fast::Hasher::new()),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        #[cfg(feature = "machine-kaonic1s")]
        if let HasherImpl::Pending(buffered) = &mut self.inner {
            if buffered.len() + data.len() < af_alg::min_hw_len() {
                if buffered.is_empty() {
                    // One allocation for the whole pending phase instead of
                    // doubling up towards the threshold.
                    buffered.reserve(af_alg::min_hw_len());
                }
                buffered.extend_from_slice(data);
                return;
            }

            // Long enough to be worth the syscalls: replay what was held back
            // into the hardware hasher, or into a software one if the socket
            // cannot be opened after all.
            let buffered = std::mem::take(buffered);
            self.inner = match af_alg::HwHasher::new() {
                Some(mut hasher) => {
                    hasher.update(&buffered);
                    hasher.update(data);
                    HasherImpl::Hardware(hasher)
                }
                None => {
                    let mut hasher = crc32fast::Hasher::new();
                    hasher.update(&buffered);
                    hasher.update(data);
                    HasherImpl::Software(hasher)
                }
            };
            return;
        }

        match &mut self.inner {
            HasherImpl::Software(hasher) => hasher.update(data),
            #[cfg(feature = "machine-kaonic1s")]
            HasherImpl::Hardware(hasher) => hasher.update(data),
            #[cfg(feature = "machine-kaonic1s")]
            HasherImpl::Pending(_) => unreachable!("handled above"),
        }
    }

    pub fn finalize(self) -> u32 {
        match self.inner {
            HasherImpl::Software(hasher) => hasher.finalize(),
            #[cfg(feature = "machine-kaonic1s")]
            HasherImpl::Pending(buffered) => software::crc32(&buffered),
            #[cfg(feature = "machine-kaonic1s")]
            HasherImpl::Hardware(hasher) => hasher.finalize(),
        }
    }
}

impl Default for Crc32Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: &[(&[u8], u32)] = &[
        (b"", 0x0000_0000),
        (b"123456789", 0xCBF4_3926),
        (
            b"The quick brown fox jumps over the lazy dog",
            0x414F_A339,
        ),
    ];

    #[test]
    fn known_vectors() {
        for &(data, expected) in VECTORS {
            assert_eq!(crc32(data), expected, "one-shot crc32 of {:?}", data);
        }
    }

    #[test]
    fn streaming_matches_oneshot() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let expected = crc32(data);

        for split in 0..=data.len() {
            let mut hasher = Crc32Hasher::new();
            hasher.update(&data[..split]);
            hasher.update(&data[split..]);
            assert_eq!(hasher.finalize(), expected, "split at {}", split);
        }
    }

    #[test]
    fn matches_reference() {
        let data: Vec<u8> = (0..4096u32).map(|i| (i * 31 + 7) as u8).collect();
        assert_eq!(crc32(&data), crc32fast::hash(&data));
    }

    #[test]
    fn streaming_crosses_hardware_threshold() {
        // Exercises the pending -> hardware/software handover with chunks on
        // both sides of the threshold.
        let data: Vec<u8> = (0..16 * 1024u32).map(|i| (i * 17 + 3) as u8).collect();
        let expected = crc32fast::hash(&data);

        for chunk in [1usize, 7, 64, 512, 4096] {
            let mut hasher = Crc32Hasher::new();
            for part in data.chunks(chunk) {
                hasher.update(part);
            }
            assert_eq!(hasher.finalize(), expected, "chunk size {}", chunk);
        }
    }
}

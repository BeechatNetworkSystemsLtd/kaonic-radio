//! Hardware CRC32 backend using the Linux kernel crypto API (AF_ALG).
//!
//! On kaonic1s the kernel "crc32" hash is served by the STM32 CRC1
//! peripheral (drivers/crypto/stm32/stm32-crc32.c, priority 200); when the
//! peripheral is absent the generic implementation (priority 100) answers
//! instead — still correct, just not accelerated.
//!
//! Kernel "crc32" shash semantics differ from CRC-32/ISO-HDLC:
//! - the seed comes from the socket key and defaults to 0, so the standard
//!   init value 0xFFFFFFFF must be set with ALG_SET_KEY
//! - the final XOR with 0xFFFFFFFF is not applied by the kernel
//! - the digest is the internal u32 state in little-endian byte order
//!
//! Both points are handled by [`normalize_digest`] and verified empirically
//! by a known-answer check during the probe: if the kernel disagrees with
//! the reference vector for any reason, the backend reports itself
//! unavailable and the crate stays on software CRC.
//!
//! Fork caveat: operation sockets carry in-kernel hash state and survive
//! `fork()` (`SOCK_CLOEXEC` only covers exec). A process that forks while a
//! checksum is in flight and then hashes from both sides of the fork on the
//! same inherited fd would interleave state. No current caller does this;
//! if one appears, it must re-`init()` in the child.

use std::cell::RefCell;
use std::io;
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::software;

/// Standard CRC-32/ISO-HDLC init value, passed to the kernel as the key.
const CRC32_INIT_KEY: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// Known-answer probe vector: CRC-32/ISO-HDLC("123456789").
const KAT_INPUT: &[u8] = b"123456789";
const KAT_EXPECTED: u32 = 0xCBF4_3926;

/// AF_ALG sockets accept arbitrarily large sends, but stay conservative and
/// feed the kernel in bounded chunks.
const SEND_CHUNK: usize = 64 * 1024;

/// Below this many bytes a checksum is cheaper in software: an AF_ALG round
/// trip has a near-constant syscall cost regardless of payload size, while
/// software CRC scales with size. Inputs under this length are answered by
/// the software backend.
const MIN_HW_LEN: usize = 16 * 1024;

/// Bound transform socket, shared by all threads. `None` when the probe
/// failed and hardware CRC is permanently disabled for this process.
static TFM: OnceLock<Option<Tfm>> = OnceLock::new();

/// Number of checksums the peripheral actually served. Makes the threshold
/// observable: if this stays flat while traffic flows, everything is being
/// answered in software, as intended for frame-sized payloads.
static HW_OPS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Per-thread operation socket for one-shot checksums. Op sockets carry
    /// per-hash state, so they are never shared between threads.
    static OP_FD: RefCell<Option<OwnedFd>> = const { RefCell::new(None) };
}

pub fn hw_ops() -> u64 {
    HW_OPS.load(Ordering::Relaxed)
}

pub fn available() -> bool {
    tfm().is_some()
}

/// Smallest input worth sending to the hardware backend; `usize::MAX` when
/// there is no hardware backend, so callers need only one comparison to
/// decide.
pub fn min_hw_len() -> usize {
    if available() {
        MIN_HW_LEN
    } else {
        usize::MAX
    }
}

/// Opens the sockets and runs the probe now instead of on the first checksum,
/// so the first packet does not pay for it.
pub fn warm_up() {
    if let Some(tfm) = tfm() {
        OP_FD.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = tfm.accept_op().ok();
            }
        });
    }
}

/// One-shot hardware CRC32, applying the size threshold: short inputs return
/// `None` so the caller computes them in software instead of paying for two
/// syscalls. Also `None` when the backend is unavailable or a syscall failed —
/// the input is untouched, so recomputing in software is always possible.
#[inline]
pub fn crc32(data: &[u8]) -> Option<u32> {
    if data.len() < MIN_HW_LEN {
        return None;
    }

    crc32_with(tfm()?, data)
}

/// One-shot hardware CRC32 ignoring the size threshold.
pub fn crc32_unchecked(data: &[u8]) -> Option<u32> {
    crc32_with(tfm()?, data)
}

fn crc32_with(tfm: &Tfm, data: &[u8]) -> Option<u32> {
    if data.is_empty() {
        // CRC of the empty message is trivially 0; skip the syscalls.
        return None;
    }

    // Everything fallible runs inside the borrow; logging stays outside it so
    // a logger that itself computes a CRC cannot re-enter the RefCell.
    let result = OP_FD.with(|slot| {
        let mut slot = slot.borrow_mut();

        if slot.is_none() {
            *slot = Some(tfm.accept_op()?);
        }

        let op = slot.as_ref().expect("op fd populated above");
        checksum(op, data).inspect_err(|_| {
            // The op socket state is unknown after a failure; drop it so the
            // next call accepts a fresh one.
            *slot = None;
        })
    });

    match result {
        Ok(value) => {
            HW_OPS.fetch_add(1, Ordering::Relaxed);
            Some(value)
        }
        Err(err) => {
            log::debug!("kaonic-crc: AF_ALG checksum failed: {}", err);
            None
        }
    }
}

/// Streaming hardware hasher owning a dedicated operation socket, so an
/// in-progress stream cannot be corrupted by concurrent one-shot calls.
///
/// Updates are coalesced into a [`SEND_CHUNK`]-sized buffer before touching
/// the kernel: a stream fed in small chunks costs one syscall per 64 KiB, not
/// one per chunk. The buffered tail is kept so `finalize` can close the hash
/// inside its final data send — two syscalls to finish, same as one-shot.
pub struct HwHasher {
    op: OwnedFd,
    /// Coalescing buffer; never grows past [`SEND_CHUNK`].
    buf: Vec<u8>,
    /// True once any bytes reached the kernel (with MSG_MORE).
    flushed_any: bool,
    /// Set when a mid-stream syscall failed. Already-sent data cannot be
    /// recovered, so the hasher is poisoned: `finalize` returns a fixed
    /// sentinel instead of a real checksum. A packet stamped with it is
    /// accepted only if the payload's true CRC happens to equal the sentinel
    /// (a 2^-32 collision) — never because a wrong checksum was trusted.
    failed: bool,
}

impl HwHasher {
    /// Returns `None` when the hardware backend is unavailable; the caller
    /// then uses a software hasher instead.
    pub fn new() -> Option<Self> {
        let op = tfm()?
            .accept_op()
            .map_err(|err| log::debug!("kaonic-crc: AF_ALG accept failed: {}", err))
            .ok()?;

        Some(Self {
            op,
            buf: Vec::with_capacity(SEND_CHUNK),
            flushed_any: false,
            failed: false,
        })
    }

    pub fn update(&mut self, data: &[u8]) {
        if self.failed || data.is_empty() {
            return;
        }

        // Common case: the chunk fits the buffer, no syscall at all.
        if self.buf.len() + data.len() <= SEND_CHUNK {
            self.buf.extend_from_slice(data);
            return;
        }

        // It does not fit: stream the buffer plus all but a tail of `data`,
        // keeping the tail (1..=SEND_CHUNK bytes) buffered so `finalize` can
        // close the hash inside its final data send.
        let tail_len = (data.len() - 1) % SEND_CHUNK + 1;
        let (head, tail) = data.split_at(data.len() - tail_len);

        let sent = if self.buf.is_empty() {
            send_all(&self.op, head)
        } else {
            send_all(&self.op, &self.buf).and_then(|()| send_all(&self.op, head))
        };

        match sent {
            Ok(()) => {
                // The first branch guarantees buf + head is non-empty here.
                self.flushed_any = true;
                self.buf.clear();
                self.buf.extend_from_slice(tail);
            }
            Err(err) => {
                log::error!(
                    "kaonic-crc: AF_ALG stream update failed, checksum poisoned: {}",
                    err
                );
                self.failed = true;
            }
        }
    }

    pub fn finalize(self) -> u32 {
        // See the `failed` field: a fixed sentinel whose acceptance requires
        // a 2^-32 collision, never a silently trusted wrong checksum.
        const POISONED: u32 = 0xFFFF_FFFF;

        if self.failed {
            return POISONED;
        }

        let result = if !self.buf.is_empty() {
            // Buffered tail closes the hash in its own send: 2 syscalls.
            send_final(&self.op, &self.buf).and_then(|()| read_digest(&self.op))
        } else if self.flushed_any {
            // Everything already streamed; close with a zero-length send.
            finish(&self.op)
        } else {
            return software::crc32(&[]);
        };

        match result {
            Ok(value) => {
                HW_OPS.fetch_add(1, Ordering::Relaxed);
                value
            }
            Err(err) => {
                log::error!(
                    "kaonic-crc: AF_ALG stream finalize failed, checksum poisoned: {}",
                    err
                );
                POISONED
            }
        }
    }
}

/// Kernel digest -> standard CRC-32/ISO-HDLC: interpret the little-endian
/// internal state and apply the final XOR the kernel omits.
fn normalize_digest(digest: [u8; 4]) -> u32 {
    u32::from_le_bytes(digest) ^ 0xFFFF_FFFF
}

#[inline]
fn tfm() -> Option<&'static Tfm> {
    TFM.get_or_init(|| match Tfm::open().and_then(Tfm::self_check) {
        Ok(tfm) => {
            log::info!(
                "kaonic-crc: using AF_ALG hardware crc32 for inputs >= {} bytes",
                MIN_HW_LEN
            );
            Some(tfm)
        }
        Err(err) => {
            log::warn!(
                "kaonic-crc: AF_ALG crc32 unavailable ({}), falling back to software CRC",
                err
            );
            None
        }
    })
    .as_ref()
}

struct Tfm(OwnedFd);

impl Tfm {
    fn open() -> io::Result<Self> {
        let fd = unsafe {
            libc::socket(
                libc::AF_ALG,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        let mut addr: libc::sockaddr_alg = unsafe { mem::zeroed() };
        addr.salg_family = libc::AF_ALG as u16;
        addr.salg_type[..b"hash".len()].copy_from_slice(b"hash");
        addr.salg_name[..b"crc32".len()].copy_from_slice(b"crc32");

        let ret = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_alg as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_alg>() as libc::socklen_t,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        // The key is the CRC seed; without it the kernel starts from 0
        // instead of the standard 0xFFFFFFFF.
        let ret = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_ALG,
                libc::ALG_SET_KEY,
                CRC32_INIT_KEY.as_ptr() as *const libc::c_void,
                CRC32_INIT_KEY.len() as libc::socklen_t,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self(fd))
    }

    /// Startup proof that the kernel computes the same algorithm as the
    /// software backend — CRC-32/ISO-HDLC — before hardware serves a single
    /// production checksum. Three checks, all against the answers the crate
    /// would otherwise compute in software:
    ///
    /// 1. a published known-answer vector, pinning every CRC parameter
    ///    (polynomial, reflection, init, final XOR, digest endianness)
    /// 2. a pseudo-random buffer cross-checked against [`software::crc32`],
    ///    so agreement is proven on arbitrary data, not just the vector
    /// 3. the same buffer through the streaming path (MSG_MORE chunks plus
    ///    zero-length finalize), which `HwHasher` relies on
    ///
    /// Any disagreement disables hardware for the process and the crate stays
    /// on software CRC.
    fn self_check(self) -> io::Result<Self> {
        fn mismatch(what: &str, got: u32, want: u32) -> io::Error {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: got {:#010x}, expected {:#010x}", what, got, want),
            )
        }

        let op = self.accept_op()?;

        // 1. Known-answer vector.
        let value = checksum(&op, KAT_INPUT)?;
        if value != KAT_EXPECTED {
            return Err(mismatch("known-answer check failed", value, KAT_EXPECTED));
        }

        // 2. Software/hardware agreement on pseudo-random data (xorshift32,
        // fixed seed, so every boot checks the identical buffer).
        let mut buf = [0u8; 256];
        let mut state: u32 = 0x2CDA_7E15;
        for byte in buf.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let want = software::crc32(&buf);

        let value = checksum(&op, &buf)?;
        if value != want {
            return Err(mismatch("backend cross-check failed", value, want));
        }

        // 3. Same buffer via the streaming path.
        send_all(&op, &buf[..100])?;
        send_all(&op, &buf[100..])?;
        let value = finish(&op)?;
        if value != want {
            return Err(mismatch("streaming cross-check failed", value, want));
        }

        Ok(self)
    }

    fn accept_op(&self) -> io::Result<OwnedFd> {
        let fd = unsafe {
            libc::accept4(
                self.0.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// One-shot checksum in the fewest syscalls the kernel allows: the data send
/// itself closes the hash (no MSG_MORE), so the common case is `send` + `read`
/// rather than `send` + zero-length `send` + `read`.
fn checksum(op: &OwnedFd, data: &[u8]) -> io::Result<u32> {
    debug_assert!(!data.is_empty());

    if data.len() <= SEND_CHUNK {
        send_final(op, data)?;
    } else {
        // Oversized input: everything but the tail is streamed, and the tail
        // closes the hash.
        let split = data.len() - SEND_CHUNK;
        send_all(op, &data[..split])?;
        send_final(op, &data[split..])?;
    }

    read_digest(op)
}

/// Feed `data` into the hash; the operation stays open (MSG_MORE on every
/// chunk) until it is closed by [`send_final`] or [`finish`].
fn send_all(op: &OwnedFd, data: &[u8]) -> io::Result<()> {
    for chunk in data.chunks(SEND_CHUNK) {
        let mut offset = 0;
        while offset < chunk.len() {
            let ret = unsafe {
                libc::send(
                    op.as_raw_fd(),
                    chunk[offset..].as_ptr() as *const libc::c_void,
                    chunk.len() - offset,
                    libc::MSG_MORE,
                )
            };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            offset += ret as usize;
        }
    }

    Ok(())
}

/// Send the last bytes of a hash without MSG_MORE, which both feeds and
/// closes the operation.
///
/// The kernel finalizes after copying whatever it managed to take, so a short
/// send would finalize over a truncated message. That only happens on signals
/// or memory pressure; treat it as an error rather than trying to recover, and
/// the caller falls back to a software checksum over the untouched input.
fn send_final(op: &OwnedFd, data: &[u8]) -> io::Result<()> {
    loop {
        let ret = unsafe {
            libc::send(
                op.as_raw_fd(),
                data.as_ptr() as *const libc::c_void,
                data.len(),
                0,
            )
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if ret as usize != data.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short final send: {} of {} bytes", ret, data.len()),
            ));
        }
        return Ok(());
    }
}

/// Close a streamed hash operation (zero-length send without MSG_MORE) and
/// read back the digest. Used by the streaming hasher, whose updates have
/// already been sent with MSG_MORE.
fn finish(op: &OwnedFd) -> io::Result<u32> {
    loop {
        let ret = unsafe { libc::send(op.as_raw_fd(), std::ptr::null(), 0, 0) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        break;
    }

    read_digest(op)
}

/// Read the 4-byte digest. The op socket resets afterwards and can be reused
/// for the next checksum.
fn read_digest(op: &OwnedFd) -> io::Result<u32> {
    let mut digest = [0u8; 4];
    loop {
        let ret = unsafe {
            libc::read(
                op.as_raw_fd(),
                digest.as_mut_ptr() as *mut libc::c_void,
                digest.len(),
            )
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if ret as usize != digest.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("short digest read: {} bytes", ret),
            ));
        }
        break;
    }

    Ok(normalize_digest(digest))
}

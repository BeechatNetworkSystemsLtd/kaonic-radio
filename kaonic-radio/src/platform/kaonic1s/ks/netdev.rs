//! Packet sockets on the driver's raw interfaces, and the wait that replaces
//! the interrupt line.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    time::Duration,
};

/// Not in every libc release; the value is part of the kernel ABI.
const PACKET_IGNORE_OUTGOING: libc::c_int = 23;

/// How much the socket may hold in frames not yet on the air. The kernel
/// doubles the figure and charges each frame its buffer (about 4.5 kB for a
/// full one), so this is roughly four full frames: enough that the driver
/// always finds the next one waiting when a transmit ends, and little enough
/// that a saturated sender is held back instead of queueing seconds of stale
/// traffic.
const SEND_BUFFER: libc::c_int = 9 * 1024;

fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

fn to_millis(timeout: Duration) -> libc::c_int {
    // Rounded up: a sub-millisecond wait must not turn into no wait at all.
    let millis = timeout.as_micros().div_ceil(1000);

    libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX)
}

fn set_option(
    fd: RawFd,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    // SAFETY: the option value is a valid int for the length given.
    cvt(unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &value as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
    .map(|_| ())
}

/// Index of a network interface, if it exists.
pub fn if_index(iface: &str) -> Option<i32> {
    let name = std::ffi::CString::new(iface).ok()?;

    // SAFETY: the name is a valid NUL-terminated string.
    match unsafe { libc::if_nametoindex(name.as_ptr()) } {
        0 => None,
        index => Some(index as i32),
    }
}

/// Keeps the kernel's own traffic off an interface.
///
/// The driver's interfaces are ordinary network devices, so when one comes up
/// the IPv6 stack gives it an address and announces it - router solicitations
/// and multicast reports, transmitted over the radio as if they were ours.
/// Nothing here speaks IP over the radio, so IPv6 is switched off on it.
pub fn disable_ipv6(iface: &str) {
    let path = format!("/proc/sys/net/ipv6/conf/{iface}/disable_ipv6");

    // Absent when the kernel has no IPv6, which is just as good.
    if let Err(err) = std::fs::write(&path, "1") {
        if err.kind() != io::ErrorKind::NotFound {
            log::warn!("{iface}: can't disable IPv6: {err}");
        }
    }
}

/// A non-blocking packet socket bound to one interface, carrying whole PHY
/// frames in both directions.
pub struct PacketSocket {
    fd: OwnedFd,
    /// The interface name as the link ioctls take it.
    name: [libc::c_char; libc::IFNAMSIZ],
}

impl PacketSocket {
    pub fn open(iface: &str) -> io::Result<Self> {
        let index = if_index(iface).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

        // Protocol zero: the socket hears nothing until it is bound. Opened
        // with a protocol it would collect frames from every interface on the
        // board in the meantime.
        let fd = cvt(unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        })?;

        // SAFETY: the descriptor was just created and is owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        // A packet socket hears both directions, so without this every frame
        // sent comes straight back as if the radio had received it. Filtered
        // in the kernel: the frame is not even cloned for this socket.
        set_option(fd.as_raw_fd(), libc::SOL_PACKET, PACKET_IGNORE_OUTGOING, 1)?;
        set_option(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            SEND_BUFFER,
        )?;

        // SAFETY: all-zero is a valid value for the structure.
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as libc::c_ushort;
        addr.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
        addr.sll_ifindex = index;

        // SAFETY: the address is fully initialised and sized correctly.
        cvt(unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        })?;

        let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
        for (slot, byte) in name.iter_mut().zip(iface.bytes().take(libc::IFNAMSIZ - 1)) {
            *slot = byte as libc::c_char;
        }

        Ok(Self { fd, name })
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Queues one frame for transmission. `WouldBlock` when the socket already
    /// holds as much as it may.
    pub fn send(&self, data: &[u8]) -> io::Result<()> {
        // SAFETY: the slice is valid for its length for the duration of the call.
        let sent = unsafe {
            libc::send(
                self.fd.as_raw_fd(),
                data.as_ptr() as *const libc::c_void,
                data.len(),
                0,
            )
        };

        if sent < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    /// Takes one frame, if there is one. Returns the length on the air, which
    /// is larger than the buffer when the frame did not fit.
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: the buffer is valid for its length for the duration of the call.
        let len = unsafe {
            libc::recv(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_TRUNC,
            )
        };

        if len < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(len as usize)
    }

    /// Blocks until the socket is readable (`POLLIN`) or has room to send
    /// (`POLLOUT`), or the timeout passes.
    pub fn wait(&self, events: libc::c_short, timeout: Duration) -> bool {
        let mut pfd = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events,
            revents: 0,
        };

        // SAFETY: one valid descriptor is described.
        let ready = unsafe { libc::poll(&mut pfd, 1, to_millis(timeout)) };

        ready > 0
    }

    /// Discards whatever is queued, including a pending link-down report.
    pub fn drain(&self) {
        let mut scratch = [0u8; 1];

        loop {
            match self.recv(&mut scratch) {
                Ok(_) => {}
                Err(err) if err.raw_os_error() == Some(libc::ENETDOWN) => {}
                Err(_) => break,
            }
        }
    }

    /// Brings the interface up or down, which is what puts the transceiver on
    /// or off the air.
    pub fn set_link(&self, up: bool) -> io::Result<()> {
        // SAFETY: all-zero is a valid value for the structure.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        request.ifr_name = self.name;

        // SAFETY: the request names the interface and the descriptor is a socket.
        cvt(unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) })?;

        // SAFETY: SIOCGIFFLAGS filled in the flags member.
        let flags = unsafe { request.ifr_ifru.ifru_flags };
        let wanted = if up {
            flags | libc::IFF_UP as libc::c_short
        } else {
            flags & !(libc::IFF_UP as libc::c_short)
        };

        if wanted == flags {
            return Ok(());
        }

        request.ifr_ifru.ifru_flags = wanted;

        // SAFETY: as above.
        cvt(unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::SIOCSIFFLAGS as _, &request) })?;

        Ok(())
    }
}

/// Wakes once for each arrival on the sockets it watches.
///
/// The interrupt line this replaces is an edge: waiting on it consumes it. A
/// socket is a level - it stays readable for as long as a frame sits unread -
/// so a thread that merely polled it would be told about the same frame over
/// and over until the reader got to it, and on a single core that thread would
/// be taking the time the reader needs. Edge-triggered epoll restores the
/// behaviour of the interrupt: one wake-up per arrival, however long the frame
/// then waits, and the thread sleeps in the kernel in between.
pub struct EdgeWaiter {
    epoll: OwnedFd,
}

impl EdgeWaiter {
    pub fn new() -> io::Result<Self> {
        let fd = cvt(unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) })?;

        // SAFETY: the descriptor was just created and is owned by nothing else.
        Ok(Self {
            epoll: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub fn watch(&self, fd: RawFd) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: fd as u64,
        };

        // SAFETY: both descriptors are open and the event is initialised.
        cvt(unsafe {
            libc::epoll_ctl(self.epoll.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut event)
        })?;

        Ok(())
    }

    /// Blocks until something arrives or the timeout passes; `None` waits
    /// indefinitely.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 2];

        // SAFETY: the buffer holds as many events as are asked for.
        let ready = unsafe {
            libc::epoll_wait(
                self.epoll.as_raw_fd(),
                events.as_mut_ptr(),
                events.len() as libc::c_int,
                timeout.map(to_millis).unwrap_or(-1),
            )
        };

        ready > 0
    }
}

//! The driver's settings, under `/sys/class/net/<iface>/rf215/`.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::FileExt,
    path::PathBuf,
};

/// The settings directory of one transceiver.
pub struct Attrs {
    dir: PathBuf,
}

impl Attrs {
    /// `None` when the interface is not one of the driver's.
    pub fn open(iface: &str) -> Option<Self> {
        let dir = PathBuf::from(format!("/sys/class/net/{iface}/rf215"));

        dir.is_dir().then_some(Self { dir })
    }

    /// Writes one setting. The driver applies it before the write returns and
    /// fails the write if the chip refuses it.
    pub fn write(&self, name: &str, value: &str) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(self.dir.join(name))?
            .write_all(value.as_bytes())
    }

    pub fn read_i32(&self, name: &str) -> io::Result<i32> {
        std::fs::read_to_string(self.dir.join(name))?
            .trim()
            .parse()
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
    }

    /// Opens an attribute to be read again and again.
    pub fn reader(&self, name: &str) -> io::Result<AttrReader> {
        Ok(AttrReader {
            file: File::open(self.dir.join(name))?,
        })
    }
}

/// An attribute kept open, for values read once per frame: a read from offset
/// zero has the kernel produce the value afresh, so each one costs a single
/// system call instead of an open, a read and a close.
pub struct AttrReader {
    file: File,
}

impl AttrReader {
    pub fn read_i32(&self) -> Option<i32> {
        let mut buf = [0u8; 16];
        let len = self.file.read_at(&mut buf, 0).ok()?;

        core::str::from_utf8(&buf[..len]).ok()?.trim().parse().ok()
    }
}

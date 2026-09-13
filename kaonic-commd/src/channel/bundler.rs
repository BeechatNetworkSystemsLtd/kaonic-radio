//! Packs several small payloads into one radio frame.
//!
//! A frame costs the same airtime whether it carries 60 bytes or 2000, so a
//! channel that produces small payloads (voice, control) waits a short window
//! and sends them together. The format is part of the channel's profile —
//! both ends run the same profile — so it needs no marker on the wire:
//!
//! ```text
//! count:u8  (len:u16 LE, bytes)*
//! ```

use std::time::{Duration, Instant};

pub struct Bundler {
    buf: Vec<u8>,
    count: u8,
    opened: Option<Instant>,
}

impl Bundler {
    const HEADER: usize = 1;
    const ENTRY: usize = 2;

    pub fn new(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        buf.push(0);
        Self {
            buf,
            count: 0,
            opened: None,
        }
    }

    /// Bytes `payload` would add to the bundle.
    pub fn cost(payload_len: usize) -> usize {
        Self::ENTRY + payload_len
    }

    /// `true` if `payload` fits within `capacity` bytes alongside what is
    /// already bundled.
    pub fn fits(&self, payload_len: usize, capacity: usize) -> bool {
        self.buf.len() + Self::cost(payload_len) <= capacity
    }

    pub fn push(&mut self, payload: &[u8], now: Instant) {
        if self.count == 0 {
            self.opened = Some(now);
        }
        self.buf
            .extend_from_slice(&(payload.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(payload);
        self.count += 1;
        self.buf[0] = self.count;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn count(&self) -> u8 {
        self.count
    }

    /// `true` once the first payload has waited `window`.
    pub fn expired(&self, window: Duration, now: Instant) -> bool {
        self.opened
            .map_or(false, |opened| now.duration_since(opened) >= window)
    }

    /// The bundle as a frame payload; leaves the bundler empty.
    pub fn take(&mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.buf.capacity());
        out.push(0);
        std::mem::swap(&mut self.buf, &mut out);
        self.count = 0;
        self.opened = None;
        out
    }

    /// Payloads inside a bundle produced by [`take`](Self::take).
    pub fn split(bundle: &[u8]) -> Split<'_> {
        Split {
            rest: bundle.get(Self::HEADER..).unwrap_or(&[]),
            remaining: bundle.first().copied().unwrap_or(0),
        }
    }
}

pub struct Split<'a> {
    rest: &'a [u8],
    remaining: u8,
}

impl<'a> Iterator for Split<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.remaining == 0 || self.rest.len() < Bundler::ENTRY {
            return None;
        }
        let len = u16::from_le_bytes([self.rest[0], self.rest[1]]) as usize;
        let body = self.rest.get(Bundler::ENTRY..Bundler::ENTRY + len)?;
        self.rest = &self.rest[Bundler::ENTRY + len..];
        self.remaining -= 1;
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_respects_capacity() {
        let mut bundler = Bundler::new(64);
        let now = Instant::now();
        assert!(bundler.fits(10, 64));
        bundler.push(b"0123456789", now);
        bundler.push(b"ab", now);
        assert_eq!(bundler.count(), 2);
        assert!(!bundler.fits(60, 64));

        let bundle = bundler.take();
        assert!(bundler.is_empty());
        let parts: Vec<&[u8]> = Bundler::split(&bundle).collect();
        assert_eq!(parts, vec![&b"0123456789"[..], &b"ab"[..]]);
    }

    #[test]
    fn a_truncated_bundle_yields_only_complete_payloads() {
        let mut bundler = Bundler::new(32);
        let now = Instant::now();
        bundler.push(b"abc", now);
        bundler.push(b"defgh", now);
        let bundle = bundler.take();
        let parts: Vec<&[u8]> = Bundler::split(&bundle[..bundle.len() - 2]).collect();
        assert_eq!(parts, vec![&b"abc"[..]]);
    }
}

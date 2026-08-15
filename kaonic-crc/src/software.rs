//! Software CRC-32/ISO-HDLC backend. Always compiled: it is the reference
//! implementation on host builds and the runtime fallback on kaonic1s.

pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

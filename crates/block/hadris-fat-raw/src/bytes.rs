//! Little-endian readers at a byte offset, shared with `hadris-fat`.
//!
//! Each reader panics if `bytes` is shorter than `at` plus the width.

/// Reads a little-endian `u16` at `at`.
#[inline]
pub fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Reads a little-endian `u32` at `at`.
#[inline]
pub fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads a little-endian `u64` at `at`.
#[inline]
pub fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::{le16, le32, le64};

    #[test]
    fn reads_at_offset() {
        let bytes = [0xAA, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(le16(&bytes, 1), 0x0201);
        assert_eq!(le32(&bytes, 1), 0x0403_0201);
        assert_eq!(le64(&bytes, 1), 0x0807_0605_0403_0201);
    }

    #[test]
    #[should_panic]
    fn short_slice_panics() {
        le64(&[0u8; 8], 1);
    }
}

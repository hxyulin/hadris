//! Joliet extension support for ISO 9660
//!
//! Joliet allows Unicode filenames (up to 64 characters) encoded as UTF-16 Big Endian.

pub static ESCAPE_SEQUNCES: [[u8; 3]; 3] = [*b"%/@", *b"%/C", *b"%/E"];

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JolietLevel {
    /// Level 1 - UCS-2 Level 1 (escape sequence %/@)
    Level1 = 1,
    /// Level 2 - UCS-2 Level 2 (escape sequence %/C)
    Level2 = 2,
    /// Level 3 - UCS-2 Level 3 (escape sequence %/E) - most common
    Level3 = 3,
}

impl JolietLevel {
    pub fn all() -> &'static [JolietLevel] {
        static LEVELS: [JolietLevel; 3] = [
            JolietLevel::Level1,
            JolietLevel::Level2,
            JolietLevel::Level3,
        ];
        &LEVELS
    }

    /// Get the escape sequence for this Joliet level
    pub fn escape_sequence(self) -> [u8; 32] {
        let mut output = [b' '; 32];
        match self {
            Self::Level1 => output[0..3].copy_from_slice(b"%/@"),
            Self::Level2 => output[0..3].copy_from_slice(b"%/C"),
            Self::Level3 => output[0..3].copy_from_slice(b"%/E"),
        }
        output
    }

    /// Try to detect the Joliet level from escape sequences
    pub fn from_escape_sequence(escape: &[u8; 32]) -> Option<Self> {
        if escape[0..3] == *b"%/@" {
            Some(Self::Level1)
        } else if escape[0..3] == *b"%/C" {
            Some(Self::Level2)
        } else if escape[0..3] == *b"%/E" {
            Some(Self::Level3)
        } else {
            None
        }
    }
}

/// Decode a Joliet filename from UTF-16 Big Endian bytes
///
/// Joliet uses UTF-16 BE encoding for filenames. This function decodes
/// the raw bytes into a String.
#[cfg(feature = "alloc")]
pub fn decode_joliet_name(bytes: &[u8]) -> alloc::string::String {
    // UTF-16 BE: each character is 2 bytes, high byte first
    if bytes.len() < 2 {
        return alloc::string::String::new();
    }

    // Remove trailing ;1 version suffix if present
    let bytes = strip_version_suffix(bytes);

    // Convert pairs of bytes to u16 code units
    let code_units: alloc::vec::Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();

    // Decode UTF-16 to String
    alloc::string::String::from_utf16_lossy(&code_units)
}

/// Strip the version suffix (;1) from a Joliet filename if present
#[cfg(feature = "alloc")]
fn strip_version_suffix(bytes: &[u8]) -> &[u8] {
    // Look for ";1" at the end (0x00 0x3B 0x00 0x31 in UTF-16 BE)
    if bytes.len() >= 4 {
        let suffix = &bytes[bytes.len() - 4..];
        if suffix == [0x00, b';', 0x00, b'1'] {
            return &bytes[..bytes.len() - 4];
        }
    }
    bytes
}

/// Encode a string as a Joliet filename (UTF-16 Big Endian)
#[cfg(feature = "alloc")]
pub fn encode_joliet_name(name: &str) -> alloc::vec::Vec<u8> {
    let mut result = alloc::vec::Vec::with_capacity(name.len() * 2);
    for c in name.encode_utf16() {
        result.extend_from_slice(&c.to_be_bytes());
    }
    result
}

/// Check if a byte slice looks like a Joliet (UTF-16 BE) filename
///
/// Returns true if the bytes appear to be valid UTF-16 BE text
pub fn is_likely_joliet_name(bytes: &[u8]) -> bool {
    // Must be even length for UTF-16
    if bytes.len() % 2 != 0 || bytes.is_empty() {
        return false;
    }

    // Check for common ASCII characters in UTF-16 BE (0x00 followed by ASCII char)
    // This is a heuristic - Joliet names often contain ASCII which appears as 0x00 XX
    let mut ascii_count = 0;
    for pair in bytes.chunks_exact(2) {
        if pair[0] == 0x00 && pair[1].is_ascii_graphic() {
            ascii_count += 1;
        }
    }

    // If more than half the characters are ASCII, likely Joliet
    ascii_count * 2 > bytes.len() / 2
}


#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;

    #[test]
    fn test_escape_sequences() {
        let level1 = b"%/@";
        assert_eq!(level1, &ESCAPE_SEQUNCES[0]);

        let level2 = b"%/C";
        assert_eq!(level2, &ESCAPE_SEQUNCES[1]);

        let level3 = b"%/E";
        assert_eq!(level3, &ESCAPE_SEQUNCES[2]);
    }

    #[test]
    fn test_joliet_level_escape_sequence() {
        let level1 = JolietLevel::Level1.escape_sequence();
        assert_eq!(&level1[0..3], b"%/@");

        let level2 = JolietLevel::Level2.escape_sequence();
        assert_eq!(&level2[0..3], b"%/C");

        let level3 = JolietLevel::Level3.escape_sequence();
        assert_eq!(&level3[0..3], b"%/E");
    }

    #[test]
    fn test_joliet_level_from_escape_sequence() {
        let mut seq = [b' '; 32];

        seq[0..3].copy_from_slice(b"%/@");
        assert_eq!(JolietLevel::from_escape_sequence(&seq), Some(JolietLevel::Level1));

        seq[0..3].copy_from_slice(b"%/C");
        assert_eq!(JolietLevel::from_escape_sequence(&seq), Some(JolietLevel::Level2));

        seq[0..3].copy_from_slice(b"%/E");
        assert_eq!(JolietLevel::from_escape_sequence(&seq), Some(JolietLevel::Level3));

        seq[0..3].copy_from_slice(b"XXX");
        assert_eq!(JolietLevel::from_escape_sequence(&seq), None);
    }

    #[test]
    fn test_joliet_level_all() {
        let levels = JolietLevel::all();
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], JolietLevel::Level1);
        assert_eq!(levels[1], JolietLevel::Level2);
        assert_eq!(levels[2], JolietLevel::Level3);
    }

    #[test]
    fn test_encode_joliet_name_ascii() {
        let encoded = encode_joliet_name("test.txt");
        // Each ASCII char becomes 2 bytes: 0x00, char
        assert_eq!(encoded.len(), 16);  // 8 chars * 2 bytes
        assert_eq!(&encoded[0..2], &[0x00, b't']);
        assert_eq!(&encoded[2..4], &[0x00, b'e']);
        assert_eq!(&encoded[4..6], &[0x00, b's']);
        assert_eq!(&encoded[6..8], &[0x00, b't']);
    }

    #[test]
    fn test_encode_joliet_name_unicode() {
        let encoded = encode_joliet_name("日本語");
        // 3 characters, each is a single BMP code point
        assert_eq!(encoded.len(), 6);  // 3 chars * 2 bytes

        // 日 = U+65E5 = [0x65, 0xE5] in UTF-16 BE
        assert_eq!(&encoded[0..2], &[0x65, 0xE5]);
    }

    #[test]
    fn test_decode_joliet_name_ascii() {
        // "test" in UTF-16 BE
        let bytes: &[u8] = &[0x00, b't', 0x00, b'e', 0x00, b's', 0x00, b't'];
        let decoded = decode_joliet_name(bytes);
        assert_eq!(decoded, "test");
    }

    #[test]
    fn test_decode_joliet_name_unicode() {
        // "日本" in UTF-16 BE
        let bytes: &[u8] = &[0x65, 0xE5, 0x67, 0x2C];
        let decoded = decode_joliet_name(bytes);
        assert_eq!(decoded, "日本");
    }

    #[test]
    fn test_decode_joliet_name_with_version_suffix() {
        // "test;1" in UTF-16 BE
        let bytes: &[u8] = &[
            0x00, b't', 0x00, b'e', 0x00, b's', 0x00, b't',
            0x00, b';', 0x00, b'1'
        ];
        let decoded = decode_joliet_name(bytes);
        assert_eq!(decoded, "test");  // Version suffix should be stripped
    }

    #[test]
    fn test_decode_joliet_name_empty() {
        let decoded = decode_joliet_name(&[]);
        assert_eq!(decoded, "");

        let decoded = decode_joliet_name(&[0x00]);  // Single byte (invalid)
        assert_eq!(decoded, "");
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = "test_file.txt";
        let encoded = encode_joliet_name(original);
        let decoded = decode_joliet_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_decode_roundtrip_unicode() {
        let original = "文档_2024.txt";
        let encoded = encode_joliet_name(original);
        let decoded = decode_joliet_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_is_likely_joliet_name_ascii() {
        // ASCII text in UTF-16 BE looks like 0x00, char
        let joliet: &[u8] = &[0x00, b't', 0x00, b'e', 0x00, b's', 0x00, b't'];
        assert!(is_likely_joliet_name(joliet));
    }

    #[test]
    fn test_is_likely_joliet_name_odd_length() {
        // Odd length bytes can't be UTF-16
        let bytes: &[u8] = &[0x00, b't', 0x00];
        assert!(!is_likely_joliet_name(bytes));
    }

    #[test]
    fn test_is_likely_joliet_name_empty() {
        assert!(!is_likely_joliet_name(&[]));
    }

    #[test]
    fn test_is_likely_joliet_name_non_ascii() {
        // Pure ISO 8859-1 / ASCII without high bytes
        let iso_name: &[u8] = b"TEST.TXT";
        assert!(!is_likely_joliet_name(iso_name));
    }
}

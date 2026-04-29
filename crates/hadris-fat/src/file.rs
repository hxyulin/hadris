use core::fmt;

use hadris_common::types::file::FixedFilename;

/// A type representing a short filename (8.3 format)
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShortFileName(FixedFilename<12>);

impl fmt::Debug for ShortFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ShortFileName")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Debug)]
pub struct CreateShortFileNameError;

impl fmt::Display for CreateShortFileNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("disallowed characters in short file name")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CreateShortFileNameError {}

impl ShortFileName {
    pub const ALLOWED_SYMBOLS: &'static [u8] = b"$%'-_@~`!(){}^#&";

    pub fn new(bytes: [u8; 11]) -> Result<Self, CreateShortFileNameError> {
        // Special case: "." and ".." directory entries
        if bytes == *b".          " {
            let mut name = FixedFilename::empty();
            name.push_byte(b'.');
            return Ok(Self(name));
        }
        if bytes == *b"..         " {
            let mut name = FixedFilename::empty();
            name.push_slice(b"..");
            return Ok(Self(name));
        }

        for byte in &bytes {
            if byte.is_ascii_uppercase()
                || Self::ALLOWED_SYMBOLS.contains(byte)
                || byte.is_ascii_digit()
                || *byte == b' '
                || *byte > 127
            {
                continue;
            }
            return Err(CreateShortFileNameError);
        }

        let mut name = FixedFilename::empty();
        name.push_slice(&bytes[0..8]);
        name.push_byte(b'.');
        name.push_slice(&bytes[8..11]);
        Ok(Self(name))
    }

    /// Get the raw 11-byte name for checksum calculation
    pub fn raw_bytes(&self) -> [u8; 11] {
        let s = self.0.as_str();
        let bytes = s.as_bytes();
        let mut result = [b' '; 11];
        // Copy the name part (before the dot)
        let dot_pos = bytes.iter().position(|&b| b == b'.').unwrap_or(8);
        let name_len = dot_pos.min(8);
        result[..name_len].copy_from_slice(&bytes[..name_len]);
        // Copy the extension part (after the dot)
        if dot_pos < bytes.len() {
            let ext_start = dot_pos + 1;
            let ext_len = (bytes.len() - ext_start).min(3);
            result[8..8 + ext_len].copy_from_slice(&bytes[ext_start..ext_start + ext_len]);
        }
        result
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Check if this short filename matches a given name (case-insensitive).
    /// Handles both padded ("TEST    .TXT") and unpadded ("TEST.TXT") formats.
    pub fn matches(&self, name: &str) -> bool {
        let raw = self.0.as_str();

        // Parse our stored name (format: "BASE    .EXT")
        let (our_base, our_ext) = if let Some(dot_pos) = raw.find('.') {
            (raw[..dot_pos].trim_end(), raw[dot_pos + 1..].trim_end())
        } else {
            (raw.trim_end(), "")
        };

        // Parse the search name
        let (search_base, search_ext) = if let Some(dot_pos) = name.rfind('.') {
            (&name[..dot_pos], &name[dot_pos + 1..])
        } else {
            (name, "")
        };

        // Compare base and extension (case-insensitive)
        our_base.eq_ignore_ascii_case(search_base) && our_ext.eq_ignore_ascii_case(search_ext)
    }

    /// Calculate the LFN checksum for this short filename.
    /// This is used to validate that LFN entries belong to this short name entry.
    pub fn lfn_checksum(&self) -> u8 {
        let name = self.raw_bytes();
        let mut sum: u8 = 0;
        for &byte in &name {
            // Rotate right and add
            sum = sum.rotate_right(1).wrapping_add(byte);
        }
        sum
    }

    /// Convert back to the raw 11-byte format for directory entries.
    #[cfg(feature = "write")]
    pub fn to_raw_bytes(&self) -> [u8; 11] {
        self.raw_bytes()
    }

    /// Generate an 8.3 short filename from a long name.
    ///
    /// Rules:
    /// - Uppercase all letters
    /// - Strip invalid characters, replace with `_`
    /// - Base name max 8 chars, extension max 3 chars
    /// - Add `~N` suffix for collisions (caller should increment suffix)
    #[cfg(feature = "write")]
    pub fn from_long_name(name: &str, suffix: u8) -> Result<Self, CreateShortFileNameError> {
        // Find the last dot for extension separation
        let (base, ext) = match name.rfind('.') {
            Some(pos) if pos > 0 => (&name[..pos], &name[pos + 1..]),
            _ => (name, ""),
        };

        // Process base name: uppercase, strip invalid chars
        let mut base_chars = [b' '; 8];
        let mut base_len = 0;
        for ch in base.chars() {
            if base_len >= 6 && suffix > 0 {
                // Leave room for ~N suffix
                break;
            }
            if base_len >= 8 {
                break;
            }
            let processed = Self::process_char(ch);
            if processed != 0 {
                base_chars[base_len] = processed;
                base_len += 1;
            }
        }

        // Add ~N suffix if needed (Microsoft-style collision handling)
        if suffix > 0 {
            if suffix <= 4 {
                // For N=1..4: use simple ~N suffix (e.g., FILENA~1)
                let max_base = 6; // leave room for ~N (2 chars)
                if base_len > max_base {
                    base_len = max_base;
                }
                base_chars[base_len] = b'~';
                base_len += 1;
                base_chars[base_len] = b'0' + suffix;
                base_len += 1;
            } else {
                // For N>4: use hash-based suffix ~HHHH where HHHH is a 4-char
                // hex hash derived from the long name + suffix, per Microsoft's
                // recommended approach for reducing collisions.
                let hash = Self::lfn_hash(name, suffix);
                let max_base = 2; // leave room for ~HHHH (5 chars) + at least 2 base chars
                if base_len > max_base {
                    base_len = max_base;
                }
                base_chars[base_len] = b'~';
                base_len += 1;
                // Write 4 hex digits
                for i in (0..4).rev() {
                    let nibble = ((hash >> (i * 4)) & 0xF) as u8;
                    base_chars[base_len] = if nibble < 10 {
                        b'0' + nibble
                    } else {
                        b'A' + nibble - 10
                    };
                    base_len += 1;
                }
            }
        }

        // Process extension: uppercase, strip invalid chars
        let mut ext_chars = [b' '; 3];
        let mut ext_len = 0;
        for ch in ext.chars() {
            if ext_len >= 3 {
                break;
            }
            let processed = Self::process_char(ch);
            if processed != 0 {
                ext_chars[ext_len] = processed;
                ext_len += 1;
            }
        }

        // Combine into 11-byte name
        let mut result = [b' '; 11];
        result[..8].copy_from_slice(&base_chars);
        result[8..11].copy_from_slice(&ext_chars);

        // Validate we have at least one character
        if base_len == 0 && ext_len == 0 {
            return Err(CreateShortFileNameError);
        }

        Self::new(result)
    }

    /// Compute a simple hash from a long filename and suffix for short name generation.
    /// Returns a 16-bit value used as a 4-hex-digit suffix.
    #[cfg(feature = "write")]
    fn lfn_hash(name: &str, suffix: u8) -> u16 {
        let mut hash: u16 = suffix as u16;
        for &b in name.as_bytes() {
            hash = hash.wrapping_mul(37).wrapping_add(b as u16);
        }
        hash
    }

    /// Process a character for short filename conversion.
    /// Returns 0 if the character should be skipped.
    #[cfg(feature = "write")]
    fn process_char(ch: char) -> u8 {
        if ch.is_ascii_alphanumeric() {
            ch.to_ascii_uppercase() as u8
        } else if Self::ALLOWED_SYMBOLS.contains(&(ch as u8)) {
            ch as u8
        } else if ch == ' ' || ch == '.' {
            // Skip spaces and extra dots (dots are handled separately)
            0
        } else if ch.is_ascii() {
            // Replace other ASCII chars with underscore
            b'_'
        } else {
            // Non-ASCII: replace with underscore
            b'_'
        }
    }
}

/// Maximum number of UTF-16 code units in a long filename (per the FAT LFN spec).
pub const LFN_MAX_UTF16_UNITS: usize = 255;

/// A Long File Name stored as UTF-16.
///
/// LFN entries on disk encode the filename in UTF-16LE. Storing the data in its
/// native form avoids two classes of bugs that previously lived here (see issue
/// #28): the buffer never holds invalid UTF-8, and surrogate pairs (characters
/// outside the Basic Multilingual Plane, e.g. emoji) are preserved correctly
/// regardless of how the pair lands across LFN entry boundaries — the conversion
/// to scalar values happens once, at access time.
#[cfg(feature = "lfn")]
#[derive(Clone, PartialEq, Eq)]
pub struct LongFileName {
    chars: [u16; LFN_MAX_UTF16_UNITS],
    len: usize,
}

#[cfg(feature = "lfn")]
impl fmt::Debug for LongFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct LossyChars<'a>(&'a LongFileName);
        impl fmt::Debug for LossyChars<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("\"")?;
                for ch in self.0.chars() {
                    fmt::Write::write_char(f, ch)?;
                }
                f.write_str("\"")
            }
        }
        f.debug_tuple("LongFileName")
            .field(&LossyChars(self))
            .finish()
    }
}

#[cfg(feature = "lfn")]
impl Default for LongFileName {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "lfn")]
impl LongFileName {
    /// Number of UTF-16 code units stored per LFN directory entry
    pub const CHARS_PER_ENTRY: usize = 13;

    /// Create a new empty LongFileName
    pub fn new() -> Self {
        Self {
            chars: [0; LFN_MAX_UTF16_UNITS],
            len: 0,
        }
    }

    /// Clear the filename
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Check if the filename is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of UTF-16 code units in the filename.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Prepend UTF-16LE characters from an LFN entry.
    /// LFN entries are stored in reverse order, so we prepend.
    /// Characters are: 5 from name1, 6 from name2, 2 from name3.
    pub fn prepend_lfn_entry(&mut self, name1: &[u8; 10], name2: &[u8; 12], name3: &[u8; 4]) {
        // Collect all 13 UTF-16LE code units
        let mut utf16_chars = [0u16; Self::CHARS_PER_ENTRY];

        // name1: 5 UTF-16LE characters (10 bytes)
        for i in 0..5 {
            utf16_chars[i] = u16::from_le_bytes([name1[i * 2], name1[i * 2 + 1]]);
        }
        // name2: 6 UTF-16LE characters (12 bytes)
        for i in 0..6 {
            utf16_chars[5 + i] = u16::from_le_bytes([name2[i * 2], name2[i * 2 + 1]]);
        }
        // name3: 2 UTF-16LE characters (4 bytes)
        for i in 0..2 {
            utf16_chars[11 + i] = u16::from_le_bytes([name3[i * 2], name3[i * 2 + 1]]);
        }

        // Find end of actual characters (0x0000 or 0xFFFF marks padding)
        let actual_len = utf16_chars
            .iter()
            .position(|&c| c == 0x0000 || c == 0xFFFF)
            .unwrap_or(Self::CHARS_PER_ENTRY);

        // Prepend code units to the existing buffer.
        let new_len = self.len + actual_len;
        if new_len > LFN_MAX_UTF16_UNITS {
            // Spec violation: silently drop the entry rather than panicking on
            // a malformed image. Matches the prior behavior.
            return;
        }
        if self.len > 0 {
            self.chars.copy_within(0..self.len, actual_len);
        }
        self.chars[..actual_len].copy_from_slice(&utf16_chars[..actual_len]);
        self.len = new_len;
    }

    /// Borrow the filename as raw UTF-16 code units.
    pub fn as_utf16(&self) -> &[u16] {
        &self.chars[..self.len]
    }

    /// Iterate over the decoded scalar values of the filename.
    ///
    /// Lone surrogates (which the spec disallows but a malformed image could
    /// contain) are reported as [`char::REPLACEMENT_CHARACTER`] (U+FFFD).
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        char::decode_utf16(self.chars[..self.len].iter().copied())
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    /// Compare the filename to a `&str` without allocating.
    pub fn eq_str(&self, s: &str) -> bool {
        self.chars().eq(s.chars())
    }
}

#[cfg(feature = "lfn")]
impl fmt::Display for LongFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for ch in self.chars() {
            fmt::Write::write_char(f, ch)?;
        }
        Ok(())
    }
}

/// Builder for accumulating LFN entries while iterating
#[cfg(feature = "lfn")]
pub struct LfnBuilder {
    /// The accumulated long filename
    pub name: LongFileName,
    /// Expected checksum (from the short name entry)
    pub checksum: u8,
    /// The sequence number we're expecting next (counting down from last entry)
    pub expected_seq: u8,
    /// Whether we're currently building an LFN
    pub building: bool,
}

#[cfg(feature = "lfn")]
impl Default for LfnBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "lfn")]
impl LfnBuilder {
    /// Bit mask for the last LFN entry marker
    pub const LAST_ENTRY_MASK: u8 = 0x40;
    /// Mask for the sequence number (bits 0-5)
    pub const SEQ_NUMBER_MASK: u8 = 0x3F;

    pub fn new() -> Self {
        Self {
            name: LongFileName::new(),
            checksum: 0,
            expected_seq: 0,
            building: false,
        }
    }

    /// Reset the builder state
    pub fn reset(&mut self) {
        self.name.clear();
        self.checksum = 0;
        self.expected_seq = 0;
        self.building = false;
    }

    /// Start building a new LFN from the first (last physical) entry
    pub fn start(&mut self, seq_number: u8, checksum: u8) {
        self.reset();
        self.building = true;
        self.checksum = checksum;
        // The sequence number indicates how many entries there are
        self.expected_seq = seq_number & Self::SEQ_NUMBER_MASK;
    }

    /// Add an LFN entry to the builder.
    /// Returns true if the entry was accepted, false if there was a sequence error.
    pub fn add_entry(
        &mut self,
        seq_number: u8,
        checksum: u8,
        name1: &[u8; 10],
        name2: &[u8; 12],
        name3: &[u8; 4],
    ) -> bool {
        let seq = seq_number & Self::SEQ_NUMBER_MASK;

        // Check sequence number
        if seq != self.expected_seq {
            self.reset();
            return false;
        }

        // Check checksum consistency
        if checksum != self.checksum {
            self.reset();
            return false;
        }

        // Add the characters
        self.name.prepend_lfn_entry(name1, name2, name3);

        // Decrement expected sequence for next entry
        self.expected_seq -= 1;

        true
    }

    /// Check if we've received all LFN entries (ready for the short name entry)
    pub fn is_complete(&self) -> bool {
        self.building && self.expected_seq == 0
    }

    /// Validate the checksum against a short name and take the built LFN
    pub fn finish(&mut self, short_name: &ShortFileName) -> Option<LongFileName> {
        if !self.is_complete() {
            self.reset();
            return None;
        }

        // Validate checksum
        if short_name.lfn_checksum() != self.checksum {
            self.reset();
            return None;
        }

        let result = core::mem::take(&mut self.name);
        self.reset();
        Some(result)
    }
}

#[cfg(all(test, feature = "lfn", feature = "alloc"))]
mod lfn_unicode_tests {
    use super::*;
    extern crate alloc;
    use alloc::string::ToString;

    /// Regression test for issue #28: lone surrogates in an LFN entry must
    /// not produce undefined behavior. Previously, the encoder produced
    /// invalid UTF-8 from lone surrogates and `as_str` then transmuted those
    /// bytes via `from_utf8_unchecked`. With UTF-16 storage, lone surrogates
    /// are surfaced as the replacement character (U+FFFD) instead.
    #[test]
    fn lone_high_surrogate_becomes_replacement_char() {
        let mut lfn = LongFileName::new();
        // Lone high surrogate 0xD800 followed by ASCII 'a'.
        let name1: [u8; 10] = [0x00, 0xD8, b'a', 0, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let name2: [u8; 12] = [0xFF; 12];
        let name3: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1, &name2, &name3);

        let s = lfn.to_string();
        assert_eq!(s, "\u{FFFD}a");
    }

    /// Regression test for issue #28: a valid surrogate pair encodes a
    /// supplementary-plane character (here, U+1F600 GRINNING FACE — emoji).
    /// Previously the encoder dropped the surrogate semantics and emitted two
    /// 3-byte sequences that are invalid UTF-8.
    #[test]
    fn valid_surrogate_pair_decodes_to_supplementary_codepoint() {
        let mut lfn = LongFileName::new();
        // U+1F600 = 0xD83D 0xDE00 in UTF-16LE.
        let name1: [u8; 10] = [0x3D, 0xD8, 0x00, 0xDE, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let name2: [u8; 12] = [0xFF; 12];
        let name3: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1, &name2, &name3);

        assert_eq!(lfn.to_string(), "\u{1F600}");
    }

    /// A surrogate pair split across two LFN entries (high in the earlier
    /// entry, low in the later one) must still decode correctly. The on-disk
    /// order is reverse, so the entry containing the LOW surrogate is read
    /// first (prepended first), then the entry containing the HIGH surrogate
    /// is prepended in front.
    #[test]
    fn surrogate_pair_split_across_entries() {
        let mut lfn = LongFileName::new();

        // Second-prepended entry (logically earlier in the filename): ends
        // with the high surrogate of U+1F600.
        let high_name1: [u8; 10] = [b'a', 0, b'b', 0, b'c', 0, b'd', 0, b'e', 0];
        let high_name2: [u8; 12] = [b'f', 0, b'g', 0, b'h', 0, b'i', 0, b'j', 0, b'k', 0];
        let high_name3: [u8; 4] = [b'l', 0, 0x3D, 0xD8]; // 0xD83D = high surrogate

        // First-prepended entry (logically later): starts with the low
        // surrogate of U+1F600.
        let low_name1: [u8; 10] = [
            0x00, 0xDE, // 0xDE00 = low surrogate
            b'm', 0, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let low_name2: [u8; 12] = [0xFF; 12];
        let low_name3: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&low_name1, &low_name2, &low_name3);
        lfn.prepend_lfn_entry(&high_name1, &high_name2, &high_name3);

        assert_eq!(lfn.to_string(), "abcdefghijkl\u{1F600}m");
    }

    /// Two-byte UTF-8 path: a code point in the 0x80..0x800 range must round
    /// through as one character.
    #[test]
    fn two_byte_utf8_codepoint() {
        let mut lfn = LongFileName::new();
        // U+00E9 (é)
        let name1: [u8; 10] = [0xE9, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let name2: [u8; 12] = [0xFF; 12];
        let name3: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1, &name2, &name3);

        assert_eq!(lfn.to_string(), "é");
    }

    /// Verify `eq_str` works without allocation against decoded characters.
    #[test]
    fn eq_str_matches_decoded_chars() {
        let mut lfn = LongFileName::new();
        // U+1F600
        let name1: [u8; 10] = [0x3D, 0xD8, 0x00, 0xDE, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let name2: [u8; 12] = [0xFF; 12];
        let name3: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1, &name2, &name3);

        assert!(lfn.eq_str("\u{1F600}"));
        assert!(!lfn.eq_str("X"));
    }
}

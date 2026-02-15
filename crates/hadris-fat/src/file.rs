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

#[derive(Debug, thiserror::Error)]
#[error("disallowed characters in short file name")]
pub struct CreateShortFileNameError;

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

        // Add ~N suffix if needed
        if suffix > 0 && base_len <= 6 {
            base_chars[base_len] = b'~';
            base_len += 1;
            if suffix < 10 {
                base_chars[base_len] = b'0' + suffix;
                base_len += 1;
            } else {
                // For suffix >= 10, use two digits
                base_chars[base_len] = b'0' + (suffix / 10);
                base_len += 1;
                if base_len < 8 {
                    base_chars[base_len] = b'0' + (suffix % 10);
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

/// Maximum number of UTF-8 bytes in a long filename (255 UTF-16 code units * 3 bytes max per code unit)
pub const LFN_MAX_BYTES: usize = 255 * 3;

/// A Long File Name stored as UTF-8
#[cfg(feature = "lfn")]
#[derive(Clone, PartialEq, Eq)]
pub struct LongFileName {
    bytes: [u8; LFN_MAX_BYTES],
    len: usize,
}

#[cfg(feature = "lfn")]
impl fmt::Debug for LongFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LongFileName").field(&self.as_str()).finish()
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
            bytes: [0; LFN_MAX_BYTES],
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

        // Convert UTF-16 to UTF-8 and prepend
        let mut temp_utf8 = [0u8; Self::CHARS_PER_ENTRY * 3]; // Max 3 bytes per char
        let mut temp_len = 0;

        for &code_unit in &utf16_chars[..actual_len] {
            temp_len += encode_utf16_to_utf8(code_unit, &mut temp_utf8[temp_len..]);
        }

        // Prepend to existing content
        if self.len > 0 {
            // Shift existing content
            let new_len = self.len + temp_len;
            if new_len <= LFN_MAX_BYTES {
                // Move existing bytes forward
                for i in (0..self.len).rev() {
                    self.bytes[i + temp_len] = self.bytes[i];
                }
                // Copy new content to beginning
                self.bytes[..temp_len].copy_from_slice(&temp_utf8[..temp_len]);
                self.len = new_len;
            }
        } else {
            // Just copy
            self.bytes[..temp_len].copy_from_slice(&temp_utf8[..temp_len]);
            self.len = temp_len;
        }
    }

    /// Get the filename as a UTF-8 string slice
    pub fn as_str(&self) -> &str {
        // Safety: we only ever store valid UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }
}

/// Encode a single UTF-16 code unit to UTF-8.
/// Returns the number of bytes written.
/// Note: This doesn't handle surrogate pairs; each code unit is treated independently.
#[cfg(feature = "lfn")]
fn encode_utf16_to_utf8(code_unit: u16, output: &mut [u8]) -> usize {
    let c = code_unit as u32;

    if c < 0x80 {
        // ASCII
        output[0] = c as u8;
        1
    } else if c < 0x800 {
        // 2-byte UTF-8
        output[0] = (0xC0 | (c >> 6)) as u8;
        output[1] = (0x80 | (c & 0x3F)) as u8;
        2
    } else {
        // 3-byte UTF-8 (BMP character)
        output[0] = (0xE0 | (c >> 12)) as u8;
        output[1] = (0x80 | ((c >> 6) & 0x3F)) as u8;
        output[2] = (0x80 | (c & 0x3F)) as u8;
        3
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

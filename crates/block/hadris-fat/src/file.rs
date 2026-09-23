use core::fmt;

use hadris_common::types::fixed::FixedBytes;

/// A type representing a short filename (8.3 format)
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShortFileName(FixedBytes<12>);

impl fmt::Debug for ShortFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tuple = f.debug_tuple("ShortFileName");
        // On-disk names are OEM-encoded; high bytes may not be valid UTF-8.
        match self.0.try_as_str() {
            Ok(name) => tuple.field(&name),
            Err(_) => tuple.field(&self.0.as_bytes()),
        };
        tuple.finish()
    }
}

#[derive(Debug)]
/// Error returned when bytes cannot form a valid FAT 8.3 filename.
pub struct CreateShortFileNameError;

impl fmt::Display for CreateShortFileNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("disallowed characters in short file name")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CreateShortFileNameError {}

impl ShortFileName {
    /// Punctuation permitted in a FAT short filename.
    pub const ALLOWED_SYMBOLS: &'static [u8] = crate::codec::short_name::ALLOWED_SYMBOLS;

    /// Creates a short filename from its space-padded 11-byte directory form.
    pub fn new(bytes: [u8; 11]) -> Result<Self, CreateShortFileNameError> {
        // Special case: "." and ".." directory entries
        if bytes == *b".          " {
            let mut name = FixedBytes::empty();
            name.push_byte(b'.');
            return Ok(Self(name));
        }
        if bytes == *b"..         " {
            let mut name = FixedBytes::empty();
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

        let mut name = FixedBytes::empty();
        name.push_slice(&bytes[0..8]);
        name.push_byte(b'.');
        name.push_slice(&bytes[8..11]);
        Ok(Self(name))
    }

    /// Get the raw 11-byte name for checksum calculation.
    ///
    /// Operates on the underlying byte buffer directly — does NOT go through
    /// [`Self::as_str`], which would panic on non-ASCII OEM bytes (e.g.
    /// CP437 0x82 for `é`). The 11-byte form is what's stored in the FAT
    /// directory entry, so byte-level access is the correct level here
    /// regardless of encoding.
    pub fn raw_bytes(&self) -> [u8; 11] {
        let bytes = self.0.as_bytes();
        let mut result = [b' '; 11];
        // Stored layout: "BASE    .EXT" — at most 8 base + dot + 3 ext.
        let dot_pos = bytes.iter().position(|&b| b == b'.').unwrap_or(bytes.len());
        let name_len = dot_pos.min(8);
        result[..name_len].copy_from_slice(&bytes[..name_len]);
        if dot_pos < bytes.len() {
            let ext_start = dot_pos + 1;
            let ext_len = (bytes.len() - ext_start).min(3);
            result[8..8 + ext_len].copy_from_slice(&bytes[ext_start..ext_start + ext_len]);
        }
        result
    }

    /// Returns the formatted short filename as a string.
    ///
    /// # Panics
    /// Panics if the name is not valid UTF-8 (possible with OEM high bytes on
    /// untrusted images). Use [`Self::try_as_str`] on untrusted images, or
    /// [`FileEntry::name`] / [`Self::matches`], which tolerate such names.
    ///
    /// [`FileEntry::name`]: crate::dir::FileEntry::name
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the formatted short filename, or an error if it is not valid
    /// UTF-8 (possible with OEM high bytes on untrusted images).
    pub fn try_as_str(&self) -> Result<&str, core::str::Utf8Error> {
        self.0.try_as_str()
    }

    /// The stored `BASE    .EXT` bytes without UTF-8 validation — short names
    /// are OEM-encoded, so high bytes are not necessarily valid UTF-8.
    #[cfg(feature = "alloc")]
    pub(crate) fn as_padded_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Returns a copy of this 8.3 name with the base and/or extension
    /// lowercased according to the Windows NT `DIR_NTRes` case flags.
    ///
    /// FAT stores 8.3 names uppercase on disk; the case flags (see
    /// [`NtCaseFlags`](crate::raw::NtCaseFlags)) record that the name was
    /// originally entered lowercase so it can be presented that way without a
    /// long-file-name entry. Only ASCII letters are re-cased; any other bytes
    /// (OEM high bytes, digits, symbols, the `.` separator) pass through
    /// unchanged.
    pub fn with_nt_case(&self, flags: crate::raw::NtCaseFlags) -> ShortFileName {
        use crate::raw::NtCaseFlags;

        fn push_maybe_lower(out: &mut FixedBytes<12>, part: &[u8], lower: bool) {
            for &byte in part {
                out.push_byte(if lower {
                    byte.to_ascii_lowercase()
                } else {
                    byte
                });
            }
        }

        let bytes = self.0.as_bytes();
        let dot = bytes.iter().position(|&b| b == b'.');
        let base_end = dot.unwrap_or(bytes.len());
        let mut out = FixedBytes::<12>::empty();
        push_maybe_lower(
            &mut out,
            &bytes[..base_end],
            flags.contains(NtCaseFlags::LOWER_BASE),
        );
        if let Some(dot) = dot {
            out.push_byte(b'.');
            push_maybe_lower(
                &mut out,
                &bytes[dot + 1..],
                flags.contains(NtCaseFlags::LOWER_EXT),
            );
        }
        ShortFileName(out)
    }

    /// Check if this short filename matches a given name (case-insensitive).
    /// Handles both padded ("TEST    .TXT") and unpadded ("TEST.TXT") formats.
    ///
    /// Operates on raw bytes: OEM high bytes in untrusted on-disk names are
    /// not necessarily valid UTF-8, and ASCII case comparison is well-defined
    /// on bytes.
    pub fn matches(&self, name: &str) -> bool {
        fn trim_end_spaces(mut bytes: &[u8]) -> &[u8] {
            while let [rest @ .., b' '] = bytes {
                bytes = rest;
            }
            bytes
        }

        let raw = self.0.as_bytes();
        // Stored format: "BASE    .EXT"
        let (our_base, our_ext) = match raw.iter().position(|&b| b == b'.') {
            Some(dot) => (&raw[..dot], &raw[dot + 1..]),
            None => (raw, &[][..]),
        };
        let (our_base, our_ext) = (trim_end_spaces(our_base), trim_end_spaces(our_ext));

        let name = name.as_bytes();
        let (search_base, search_ext) = match name.iter().rposition(|&b| b == b'.') {
            Some(dot) => (&name[..dot], &name[dot + 1..]),
            None => (name, &[][..]),
        };

        our_base.eq_ignore_ascii_case(search_base) && our_ext.eq_ignore_ascii_case(search_ext)
    }

    /// Calculate the LFN checksum for this short filename.
    /// This is used to validate that LFN entries belong to this short name entry.
    pub fn lfn_checksum(&self) -> u8 {
        crate::codec::lfn::checksum(&self.raw_bytes())
    }

    /// Convert back to the raw 11-byte format for directory entries.
    #[cfg(all(feature = "write", feature = "alloc"))]
    pub fn to_raw_bytes(&self) -> [u8; 11] {
        self.raw_bytes()
    }

    /// Generate an 8.3 short filename from a long name.
    ///
    /// Rules:
    /// - Uppercase all ASCII letters
    /// - Strip invalid characters, replace with `_`
    /// - Base name max 8 chars, extension max 3 chars
    /// - Add `~N` suffix for collisions (caller should increment suffix)
    ///
    /// Non-ASCII characters always become `_`. To preserve OEM-encoded Latin
    /// characters (e.g. `é` → CP437 0x82), use [`from_long_name_with`].
    ///
    /// [`from_long_name_with`]: Self::from_long_name_with
    #[cfg(all(feature = "write", feature = "alloc"))]
    pub fn from_long_name(name: &str, suffix: u8) -> Result<Self, CreateShortFileNameError> {
        Self::from_long_name_with(name, suffix, &crate::oem::LossyAsciiOemCpConverter)
    }

    /// Like [`from_long_name`](Self::from_long_name), but routes non-ASCII
    /// characters through the supplied [`OemCpConverter`](crate::oem::OemCpConverter) rather than
    /// dropping them to `_`.
    ///
    /// Characters the converter cannot encode still become `_`.
    #[cfg(all(feature = "write", feature = "alloc"))]
    pub fn from_long_name_with(
        name: &str,
        suffix: u8,
        oem: &dyn crate::oem::OemCpConverter,
    ) -> Result<Self, CreateShortFileNameError> {
        crate::codec::short_name::generate(name, suffix, |ch| oem.encode(ch))
            .ok_or(CreateShortFileNameError)
            .and_then(Self::new)
    }
}

/// Maximum number of UTF-16 code units in a long filename (per the FAT LFN spec).
pub const LFN_MAX_UTF16_UNITS: usize = crate::codec::lfn::MAX_UNITS;

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
    pub const CHARS_PER_ENTRY: usize = crate::codec::lfn::UNITS_PER_ENTRY;

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
        let (utf16_chars, actual_len) = crate::codec::lfn::unpack(name1, name2, name3);

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

    /// Compare the filename to a `&str` without allocating, using Unicode
    /// uppercase mappings for the case-insensitive FAT name lookup.
    pub(crate) fn eq_str_ignore_case(&self, s: &str) -> bool {
        self.chars()
            .flat_map(char::to_uppercase)
            .eq(s.chars().flat_map(char::to_uppercase))
    }

    /// Encode `name` as UTF-16LE into the buffer. Returns `None` if the name
    /// exceeds [`LFN_MAX_UTF16_UNITS`] (the FAT spec cap for LFN names).
    /// Used by the write path to remember the name in-memory after creating
    /// a file; the chars stay in UTF-16 so the disk-side LFN write can pull
    /// them out without a second UTF-8 conversion.
    #[cfg(all(feature = "write", feature = "alloc"))]
    pub fn from_str_utf16(name: &str) -> Option<Self> {
        let mut out = Self::new();
        for ch in name.chars() {
            let mut tmp = [0u16; 2];
            for &c in ch.encode_utf16(&mut tmp).iter() {
                if out.len >= LFN_MAX_UTF16_UNITS {
                    return None;
                }
                out.chars[out.len] = c;
                out.len += 1;
            }
        }
        Some(out)
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
    pub const LAST_ENTRY_MASK: u8 = crate::codec::lfn::LAST_ENTRY;
    /// Mask for the sequence number (bits 0-5)
    pub const SEQ_NUMBER_MASK: u8 = crate::codec::lfn::SEQUENCE_MASK;

    /// Creates an empty long-file-name sequence builder.
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
        // The sequence number indicates how many entries there are; a count
        // of zero is invalid and would underflow the countdown in add_entry.
        let count = seq_number & Self::SEQ_NUMBER_MASK;
        if count == 0 {
            return;
        }
        self.building = true;
        self.checksum = checksum;
        self.expected_seq = count;
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

        // Check sequence number; zero is not a valid LFN sequence count
        if seq == 0 || seq != self.expected_seq {
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

    #[test]
    fn eq_str_ignore_case_uses_unicode_case_mapping() {
        let mut lfn = LongFileName::new();
        let name1 = [0xc3, 0x03, b'.', 0, b't', 0, b'x', 0, b't', 0];
        lfn.prepend_lfn_entry(&name1, &[0xff; 12], &[0xff; 4]);
        assert!(lfn.eq_str_ignore_case("Σ.TXT"));
        assert!(!lfn.eq_str_ignore_case("Σ.BIN"));
    }

    #[cfg(all(feature = "write", feature = "alloc"))]
    #[test]
    fn leading_dots_produce_a_nonempty_short_basename() {
        let short = ShortFileName::from_long_name("..dots", 0).unwrap();
        assert_eq!(short.raw_bytes(), *b"DOTS       ");
    }

    /// `LfnBuilder`, driven the way directory iteration drives it, and the
    /// shared `codec::lfn::Assembler` accept and reject the same sequences.
    #[test]
    fn builder_matches_shared_assembler() {
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let short = ShortFileName::new(*b"LONGFI~1TXT").unwrap();
        let sum = short.lfn_checksum();
        for _ in 0..2000 {
            let mut builder = LfnBuilder::new();
            let mut assembler = crate::codec::lfn::Assembler::new();
            for _ in 0..(next() % 6) {
                let r = next();
                let seq = match r % 4 {
                    0 => 0x40 | ((r >> 8) as u8 % 4),
                    _ => (r >> 8) as u8 % 5,
                };
                let checksum = if r % 7 == 0 { sum ^ 1 } else { sum };
                let unit = [b'a' + (r >> 16) as u8 % 26, 0];
                let name1 = [unit[0], 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
                if seq & LfnBuilder::LAST_ENTRY_MASK != 0 {
                    builder.start(seq, checksum);
                }
                if builder.building {
                    builder.add_entry(seq, checksum, &name1, &[0xFF; 12], &[0xFF; 4]);
                }
                assembler.push(seq, checksum, &name1, &[0xFF; 12], &[0xFF; 4]);
            }
            let expected = builder.finish(&short);
            let actual = assembler.finish(sum);
            assert_eq!(expected.as_ref().map(LongFileName::as_utf16), actual);
        }
    }

    /// Regression test: an LFN entry with the last-entry flag set but a
    /// sequence count of zero is invalid; starting a sequence with it would
    /// underflow the countdown in `add_entry` (fuzz-found panic).
    #[test]
    fn lfn_start_rejects_zero_sequence_count() {
        let mut builder = LfnBuilder::new();
        builder.start(0x40, 0);
        assert!(!builder.building);
        assert!(!builder.add_entry(0x40, 0, &[0; 10], &[0; 12], &[0; 4]));
    }
}

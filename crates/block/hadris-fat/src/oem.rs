//! OEM code page conversion for short (8.3) filenames.
//!
//! FAT short names are stored as 11 raw bytes interpreted in an OEM code page
//! (originally CP437 on DOS). The default [`LossyAsciiOemCpConverter`]
//! matches the historical behavior of this crate: ASCII passes through, every
//! non-ASCII codepoint becomes `_`. [`Cp437OemCpConverter`] preserves the
//! Western-European Latin set commonly seen on physical-media images.
//!
//! `FatFs` takes a [`CodePage`] type parameter instead.
//!
//! Implementations are stateless and `Sync` so they can be installed
//! per-`FatVolume` instance and used freely from any thread.

use crate::code_page::{CodePage, Cp437};

/// Pluggable OEM code page used to encode/decode short (8.3) filename bytes.
/// See [`crate::time::TimeProvider`] for why this is `Sync`: `FatVolume` keeps
/// the converter as a `&'static dyn OemCpConverter` and would otherwise not be
/// `Send`.
pub trait OemCpConverter: core::fmt::Debug + Sync {
    /// Encode a Unicode scalar to a single OEM byte, or `None` if the
    /// character is not representable.
    ///
    /// Callers (notably [`crate::file::ShortFileName::from_long_name_with`])
    /// substitute `_` for any character this returns `None` for.
    fn encode(&self, ch: char) -> Option<u8>;

    /// Decode an OEM byte to its Unicode scalar.
    ///
    /// Implementations should always return *some* `char` (use
    /// `'\u{FFFD}'` for unmappable bytes) since callers display the result
    /// directly without further filtering.
    fn decode(&self, byte: u8) -> char;
}

/// Identity-on-ASCII converter; everything else collapses to `_` / U+FFFD.
///
/// This is the default and matches what `ShortFileName::from_long_name` did
/// before the trait existed. Cheapest possible converter — no tables, no
/// lookups, suitable for code-size-sensitive embedded builds.
#[derive(Debug, Default, Clone, Copy)]
pub struct LossyAsciiOemCpConverter;

impl OemCpConverter for LossyAsciiOemCpConverter {
    fn encode(&self, ch: char) -> Option<u8> {
        if ch.is_ascii() && !ch.is_ascii_control() {
            Some(ch as u8)
        } else {
            None
        }
    }

    fn decode(&self, byte: u8) -> char {
        if byte < 0x80 {
            byte as char
        } else {
            char::REPLACEMENT_CHARACTER
        }
    }
}

/// Default converter used when none is set on a `FatVolume`.
///
/// Always [`LossyAsciiOemCpConverter`] — the most embedded-friendly choice
/// (no tables, ASCII passes through, everything else collapses to `_`).
pub static DEFAULT_OEM_CONVERTER: LossyAsciiOemCpConverter = LossyAsciiOemCpConverter;

/// IBM CP437 converter (DOS / original FAT default code page).
///
/// Covers the Latin-1 supplement, line-drawing characters, Greek/maths
/// symbols, etc. Bytes < 0x80 are identity-mapped; bytes 0x80..=0xFF map
/// through a 128-entry table.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cp437OemCpConverter;

impl OemCpConverter for Cp437OemCpConverter {
    fn encode(&self, ch: char) -> Option<u8> {
        if ch.is_ascii() && !ch.is_ascii_control() {
            return Some(ch as u8);
        }
        CodePage::encode(&Cp437, ch)
    }

    fn decode(&self, byte: u8) -> char {
        if byte < 0x80 {
            byte as char
        } else {
            CodePage::decode(&Cp437, byte)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trips_through_lossy() {
        let c = LossyAsciiOemCpConverter;
        assert_eq!(c.encode('A'), Some(b'A'));
        assert_eq!(c.decode(b'A'), 'A');
    }

    #[test]
    fn lossy_drops_non_ascii() {
        let c = LossyAsciiOemCpConverter;
        assert_eq!(c.encode('é'), None);
        assert_eq!(c.decode(0x82), char::REPLACEMENT_CHARACTER);
    }

    #[test]
    fn cp437_round_trips_latin_supplement() {
        let c = Cp437OemCpConverter;
        for ch in ['ü', 'é', 'ä', 'Ñ', 'ß', '½', 'π'] {
            let byte = c
                .encode(ch)
                .unwrap_or_else(|| panic!("CP437 should encode {ch:?}"));
            assert_eq!(c.decode(byte), ch);
        }
    }

    #[test]
    fn cp437_passes_ascii_unchanged() {
        let c = Cp437OemCpConverter;
        assert_eq!(c.encode('A'), Some(b'A'));
        assert_eq!(c.decode(b'A'), 'A');
    }

    #[test]
    fn cp437_rejects_unmapped_codepoints() {
        // A character in neither the ASCII range nor the CP437 high table.
        // Use U+1F600 (emoji) which is well outside CP437.
        let c = Cp437OemCpConverter;
        assert_eq!(c.encode('\u{1F600}'), None);
    }
}

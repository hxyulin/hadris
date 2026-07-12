//! OEM code page conversion for short (8.3) filenames.
//!
//! FAT short names are stored as 11 raw bytes interpreted in an OEM code page
//! (originally CP437 on DOS). The default [`LossyAsciiOemCpConverter`]
//! matches the historical behavior of this crate: ASCII passes through, every
//! non-ASCII codepoint becomes `_`. [`Cp437OemCpConverter`] preserves the
//! Western-European Latin set commonly seen on physical-media images.
//!
//! Implementations are stateless and `Sync` so they can be installed
//! per-`FatFs` instance and used freely from any thread.

/// Pluggable OEM code page used to encode/decode short (8.3) filename bytes.
pub trait OemCpConverter: core::fmt::Debug {
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

/// Default converter used when none is set on a `FatFs`.
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
        // Linear search of the 128-entry high half. CP437 has no consistent
        // numerical ordering vs. Unicode, so a table-scan is the simplest
        // correct encode. (Encoding from a long name happens at most a
        // handful of times per file create — not a hot path.)
        CP437_HIGH
            .iter()
            .position(|&c| c == ch)
            .map(|i| 0x80u8 + i as u8)
    }

    fn decode(&self, byte: u8) -> char {
        if byte < 0x80 {
            byte as char
        } else {
            CP437_HIGH[(byte - 0x80) as usize]
        }
    }
}

/// CP437 mapping for bytes 0x80..=0xFF.
///
/// Source: <https://en.wikipedia.org/wiki/Code_page_437>.
const CP437_HIGH: [char; 128] = [
    // 0x80..0x8F
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    // 0x90..0x9F
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    // 0xA0..0xAF
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    // 0xB0..0xBF
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    // 0xC0..0xCF
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    // 0xD0..0xDF
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    // 0xE0..0xEF
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    // 0xF0..0xFF
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
];

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

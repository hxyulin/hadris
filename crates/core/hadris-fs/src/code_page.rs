/// The OEM code page of FAT short names: maps the bytes above `0x7F` to and
/// from characters.
///
/// A short (8.3) name is 11 bytes in an OEM code page. Bytes up to `0x7F`
/// are ASCII in every code page FAT uses, so a code page maps only the bytes
/// above. [`MountOptions::with_code_page`](crate::MountOptions::with_code_page)
/// chooses one; [`Cp437`], the original IBM PC code page, is the default.
pub trait CodePage: Send + Sync {
    /// The character for `byte`, which is above `0x7F`. Return U+FFFD for a
    /// byte the code page leaves undefined.
    fn decode(&self, byte: u8) -> char;

    /// The byte above `0x7F` for `ch`, which is not ASCII, or `None` when
    /// the code page has none. A generated short name holds `_` in its
    /// place. Bytes up to `0x7F` are treated as `None`.
    fn encode(&self, ch: char) -> Option<u8>;
}

impl<P: CodePage + ?Sized> CodePage for &P {
    fn decode(&self, byte: u8) -> char {
        (**self).decode(byte)
    }

    fn encode(&self, ch: char) -> Option<u8> {
        (**self).encode(ch)
    }
}

/// ASCII only. Needs no table.
///
/// A byte `b` above `0x7F` reads as the private-use character
/// `U+F700 + b` (U+F780 to U+F7FF), so every short name has its own
/// name, and that name finds the entry again. Those characters encode back
/// to their bytes; every other non-ASCII character becomes `_` in generated
/// short names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Ascii;

/// The first of the private-use characters [`Ascii`] reads high bytes as.
const ASCII_ESCAPE: u32 = 0xF700;

impl CodePage for Ascii {
    fn decode(&self, byte: u8) -> char {
        match byte {
            0x80.. => {
                char::from_u32(ASCII_ESCAPE + byte as u32).unwrap_or(char::REPLACEMENT_CHARACTER)
            }
            _ => char::REPLACEMENT_CHARACTER,
        }
    }

    fn encode(&self, ch: char) -> Option<u8> {
        (ch as u32)
            .checked_sub(ASCII_ESCAPE)
            .and_then(|byte| u8::try_from(byte).ok())
            .filter(|&byte| byte >= 0x80)
    }
}

/// IBM code page 437, the code page of DOS and the FAT default: Latin
/// letters with diacritics, box drawing, Greek letters and symbols.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Cp437;

impl CodePage for Cp437 {
    fn decode(&self, byte: u8) -> char {
        match byte.checked_sub(0x80) {
            Some(index) => CP437_HIGH[index as usize],
            None => char::REPLACEMENT_CHARACTER,
        }
    }

    fn encode(&self, ch: char) -> Option<u8> {
        CP437_HIGH
            .iter()
            .position(|&c| c == ch)
            .map(|index| 0x80 + index as u8)
    }
}

/// CP437 bytes `0x80..=0xFF`, from <https://en.wikipedia.org/wiki/Code_page_437>.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{A0}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_escapes_high_bytes_to_private_use_characters() {
        for byte in 0x80..=0xFF {
            let ch = Ascii.decode(byte);
            assert_eq!(ch as u32, 0xF700 + byte as u32);
            assert_eq!(Ascii.encode(ch), Some(byte));
        }
        assert_eq!(Ascii.decode(0x41), char::REPLACEMENT_CHARACTER);
        assert_eq!(Ascii.encode('\u{E9}'), None);
        assert_eq!(Ascii.encode('\u{F741}'), None);
        assert_eq!(Ascii.encode('\u{F800}'), None);
    }

    #[test]
    fn cp437_round_trips_its_high_half() {
        for byte in 0x80..=0xFF {
            assert_eq!(Cp437.encode(Cp437.decode(byte)), Some(byte));
        }
        assert_eq!(Cp437.decode(0x82), '\u{E9}');
        assert_eq!(Cp437.encode('\u{1F600}'), None);
        assert_eq!(Cp437.decode(0x41), char::REPLACEMENT_CHARACTER);
    }

    #[test]
    fn references_are_code_pages() {
        assert_eq!(<&Cp437 as CodePage>::encode(&&Cp437, '\u{DF}'), Some(0xE1));
    }
}

//! Name comparison, case folding and UTF-16 decoding.

/// Folds one UTF-16 code unit to upper case for name comparison, ASCII
/// letters only. Needs no Unicode tables.
pub const fn fold_ascii(unit: u16) -> u16 {
    if unit >= b'a' as u16 && unit <= b'z' as u16 {
        unit - 0x20
    } else {
        unit
    }
}

/// Folds one UTF-16 code unit to upper case for name comparison, as
/// Windows does: a character of the Basic Multilingual Plane with a
/// one-character uppercase form in the plane becomes that form, and every
/// other unit, surrogates included, stays as it is. Links the Unicode case
/// tables of `core`.
pub fn fold_unicode(unit: u16) -> u16 {
    if unit < 0x80 {
        return fold_ascii(unit);
    }
    let Some(ch) = char::from_u32(unit as u32) else {
        return unit;
    };
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => u16::try_from(single as u32).unwrap_or(unit),
        _ => unit,
    }
}

/// Whether two names in UTF-16 are equal after folding each code unit with
/// `fold`, such as [`fold_ascii`] or [`fold_unicode`].
pub fn eq_folded(
    a: impl IntoIterator<Item = u16>,
    b: impl IntoIterator<Item = u16>,
    fold: fn(u16) -> u16,
) -> bool {
    a.into_iter().map(fold).eq(b.into_iter().map(fold))
}

/// The characters of UTF-16 code units, with unpaired surrogates replaced by
/// U+FFFD.
pub fn utf16_chars(units: impl IntoIterator<Item = u16>) -> impl Iterator<Item = char> {
    char::decode_utf16(units).map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER))
}

/// Writes `units` as UTF-8 into `out` and returns the length, or `None` when
/// `out` is too small.
pub fn utf16_to_utf8(units: impl IntoIterator<Item = u16>, out: &mut [u8]) -> Option<usize> {
    let mut len = 0;
    for ch in utf16_chars(units) {
        let end = len + ch.len_utf8();
        ch.encode_utf8(out.get_mut(len..end)?);
        len = end;
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_code_units() {
        assert_eq!(fold_ascii(b'a' as u16), b'A' as u16);
        assert_eq!(fold_ascii(0xE9), 0xE9);
        assert_eq!(fold_unicode(b'z' as u16), b'Z' as u16);
        assert_eq!(fold_unicode(0xE9), 0xC9);
        assert_eq!(fold_unicode(0xDF), 0xDF);
        assert_eq!(fold_unicode(0xD801), 0xD801);
        assert_eq!(fold_unicode(0x0149), 0x0149);
    }

    #[test]
    fn folds_case() {
        let eq = |a: &str, b: &str, fold: fn(u16) -> u16| {
            eq_folded(a.encode_utf16(), b.encode_utf16(), fold)
        };
        assert!(eq("Readme.TXT", "README.txt", fold_ascii));
        assert!(eq("\u{E9}t\u{E9}", "\u{C9}T\u{C9}", fold_unicode));
        assert!(!eq("\u{E9}t\u{E9}", "\u{C9}T\u{C9}", fold_ascii));
        assert!(!eq("abc", "abcd", fold_unicode));
        assert!(!eq("\u{10428}", "\u{10400}", fold_unicode));
    }

    #[test]
    fn decodes_utf16() {
        let units: std::vec::Vec<u16> = "a\u{1F600}\u{E9}".encode_utf16().collect();
        let mut out = [0u8; 16];
        let len = utf16_to_utf8(units.iter().copied(), &mut out).unwrap();
        assert_eq!(&out[..len], "a\u{1F600}\u{E9}".as_bytes());
        assert_eq!(utf16_to_utf8(units.iter().copied(), &mut out[..3]), None);
        let lone = [0xD800u16, b'x' as u16];
        let len = utf16_to_utf8(lone, &mut out).unwrap();
        assert_eq!(&out[..len], "\u{FFFD}x".as_bytes());
    }
}

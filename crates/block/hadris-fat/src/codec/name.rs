//! Name comparison and UTF-16 decoding.

/// The case folding lookups use: the one-character uppercase mapping, or the
/// character itself when its uppercase form has several characters.
pub(crate) fn fold(ch: char) -> char {
    if ch.is_ascii() {
        return ch.to_ascii_uppercase();
    }
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => ch,
    }
}

/// Whether two names are equal after [`fold`].
pub(crate) fn eq_ignore_case(
    a: impl IntoIterator<Item = char>,
    b: impl IntoIterator<Item = char>,
) -> bool {
    a.into_iter().map(fold).eq(b.into_iter().map(fold))
}

/// The characters of UTF-16 code units, with unpaired surrogates replaced by
/// U+FFFD.
pub(crate) fn utf16_chars(units: impl IntoIterator<Item = u16>) -> impl Iterator<Item = char> {
    char::decode_utf16(units).map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER))
}

/// Writes `units` as UTF-8 into `out` and returns the length, or `None` when
/// `out` is too small.
pub(crate) fn utf16_to_utf8(units: impl IntoIterator<Item = u16>, out: &mut [u8]) -> Option<usize> {
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
    fn folds_case() {
        assert!(eq_ignore_case("Readme.TXT".chars(), "README.txt".chars()));
        assert!(eq_ignore_case(
            "\u{E9}t\u{E9}".chars(),
            "\u{C9}T\u{C9}".chars()
        ));
        assert!(!eq_ignore_case("abc".chars(), "abcd".chars()));
        assert_eq!(fold('\u{DF}'), '\u{DF}');
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

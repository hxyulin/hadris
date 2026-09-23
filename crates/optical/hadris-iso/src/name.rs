//! Names as the reader presents them.

/// The identifier without a `;N` version suffix, and without the `.` that
/// ends a name with an empty extension once the version is gone.
pub(crate) fn strip_version(raw: &[u8]) -> &[u8] {
    let Some(pos) = raw.iter().rposition(|&byte| byte == b';') else {
        return raw;
    };
    if pos + 1 == raw.len() || !raw[pos + 1..].iter().all(u8::is_ascii_digit) {
        return raw;
    }
    let base = &raw[..pos];
    match base.split_last() {
        Some((b'.', rest)) if !rest.is_empty() => rest,
        _ => base,
    }
}

/// Writes `bytes` to `out` as a valid name: `/` and NUL become `_`, and a
/// name that would be empty, `.` or `..` gets underscores.
pub(crate) fn sanitize(bytes: &[u8], out: &mut [u8]) -> Option<usize> {
    let out = out.get_mut(..bytes.len().max(1))?;
    if bytes.is_empty() {
        out[0] = b'_';
        return Some(1);
    }
    for (dst, &byte) in out.iter_mut().zip(bytes) {
        *dst = if byte == b'/' || byte == 0 {
            b'_'
        } else {
            byte
        };
    }
    if matches!(&*out, b"." | b"..") {
        out.fill(b'_');
    }
    Some(bytes.len())
}

/// Decodes a big-endian UCS-2 identifier into UTF-8, without its `;N`
/// version. Unpaired surrogates become U+FFFD.
pub(crate) fn decode_ucs2(raw: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut units = raw.len() / 2;
    let unit = |index: usize| u16::from_be_bytes([raw[2 * index], raw[2 * index + 1]]);
    let mut digits = units;
    while digits > 0 && (u16::from(b'0')..=u16::from(b'9')).contains(&unit(digits - 1)) {
        digits -= 1;
    }
    if digits > 0 && digits < units && unit(digits - 1) == u16::from(b';') {
        units = digits - 1;
        if units > 1 && unit(units - 1) == u16::from(b'.') {
            units -= 1;
        }
    }
    let mut len = 0;
    for ch in char::decode_utf16((0..units).map(unit)) {
        let ch = ch.unwrap_or(char::REPLACEMENT_CHARACTER);
        let ch = if ch == '/' || ch == '\0' { '_' } else { ch };
        let dst = out.get_mut(len..len + ch.len_utf8())?;
        ch.encode_utf8(dst);
        len += ch.len_utf8();
    }
    if len == 0 || matches!(&out[..len], b"." | b"..") {
        let fill = len.max(1);
        out.get_mut(..fill)?.fill(b'_');
        return Some(fill);
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_empty_extensions_are_stripped() {
        assert_eq!(strip_version(b"README.TXT;1"), b"README.TXT");
        assert_eq!(strip_version(b"FILE.;1"), b"FILE");
        assert_eq!(strip_version(b"NOVER"), b"NOVER");
        assert_eq!(strip_version(b"A;B"), b"A;B");
        assert_eq!(strip_version(b".;1"), b".");
    }

    #[test]
    fn names_are_made_valid() {
        let mut out = [0u8; 8];
        assert_eq!(sanitize(b"a/b", &mut out), Some(3));
        assert_eq!(&out[..3], b"a_b");
        assert_eq!(sanitize(b"..", &mut out), Some(2));
        assert_eq!(&out[..2], b"__");
        assert_eq!(sanitize(b"toolongname", &mut out), None);
    }

    #[test]
    fn ucs2_decodes_without_version() {
        let raw: Vec<u8> = "caf\u{e9}.txt;1"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let mut out = [0u8; 16];
        let len = decode_ucs2(&raw, &mut out).unwrap();
        assert_eq!(&out[..len], "caf\u{e9}.txt".as_bytes());
        let lone = [0xD8, 0x00, 0x00, b'a'];
        let len = decode_ucs2(&lone, &mut out).unwrap();
        assert_eq!(&out[..len], "\u{fffd}a".as_bytes());
    }
}

//! OSTA Compressed Unicode (UDF 2.1.1) and d-strings (ECMA-167 1/7.2.12).

/// Writes the characters of a CS0 identifier, compression id first, to
/// `out` as UTF-8. Unpaired surrogates become U+FFFD. Returns the length,
/// or `None` for an unknown compression id or when `out` is too small.
pub(crate) fn decode_cs0(raw: &[u8], out: &mut [u8]) -> Option<usize> {
    let (&id, content) = raw.split_first()?;
    let mut len = 0;
    let mut push = |ch: char| -> Option<()> {
        let dst = out.get_mut(len..len + ch.len_utf8())?;
        ch.encode_utf8(dst);
        len += ch.len_utf8();
        Some(())
    };
    match id {
        8 | 254 => {
            for &byte in content {
                push(char::from(byte))?;
            }
        }
        16 | 255 => {
            let units = content
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
            for ch in char::decode_utf16(units) {
                push(ch.unwrap_or(char::REPLACEMENT_CHARACTER))?;
            }
        }
        _ => return None,
    }
    Some(len)
}

/// Decodes a CS0 file identifier into `out` as a name: `/` and NUL become
/// `_`, and a name that would be empty, `.` or `..` becomes underscores.
pub(crate) fn decode_name(raw: &[u8], out: &mut [u8]) -> Option<usize> {
    let len = decode_cs0(raw, out)?;
    for byte in &mut out[..len] {
        if *byte == b'/' || *byte == 0 {
            *byte = b'_';
        }
    }
    if len == 0 || matches!(&out[..len], b"." | b"..") {
        let fill = len.max(1);
        out.get_mut(..fill)?.fill(b'_');
        return Some(fill);
    }
    Some(len)
}

/// Decodes a d-string field: CS0 bytes with their length in the last byte.
pub(crate) fn decode_dstring(field: &[u8], out: &mut [u8]) -> usize {
    let Some((&len, body)) = field.split_last() else {
        return 0;
    };
    let len = usize::from(len);
    if len < 2 || len > body.len() {
        return 0;
    }
    decode_cs0(&body[..len], out).unwrap_or(0)
}

#[cfg(feature = "alloc")]
pub(crate) use encode::*;

#[cfg(feature = "alloc")]
mod encode {
    use alloc::vec::Vec;

    /// Encodes `text` as CS0: compression id 8 when every character is
    /// below U+0100, 16 otherwise.
    pub(crate) fn encode_cs0(text: &str) -> Vec<u8> {
        if text.chars().all(|ch| (ch as u32) <= 0xFF) {
            let mut out = Vec::with_capacity(text.len() + 1);
            out.push(8);
            out.extend(text.chars().map(|ch| ch as u8));
            out
        } else {
            let mut out = Vec::with_capacity(text.len() * 2 + 1);
            out.push(16);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            out
        }
    }

    /// Writes `text` into a d-string field of `field.len()` bytes, cut at a
    /// character boundary when it does not fit. Returns whether it was cut.
    pub(crate) fn write_dstring(field: &mut [u8], text: &str) -> bool {
        field.fill(0);
        if text.is_empty() || field.len() < 2 {
            return false;
        }
        let room = field.len() - 2;
        let wide = !text.chars().all(|ch| (ch as u32) <= 0xFF);
        let mut encoded = Vec::new();
        let mut cut = false;
        for ch in text.chars() {
            let mut units = [0u16; 2];
            let bytes: Vec<u8> = if wide {
                ch.encode_utf16(&mut units)
                    .iter()
                    .flat_map(|unit| unit.to_be_bytes())
                    .collect()
            } else {
                alloc::vec![ch as u8]
            };
            if encoded.len() + bytes.len() > room {
                cut = true;
                break;
            }
            encoded.extend_from_slice(&bytes);
        }
        field[0] = if wide { 16 } else { 8 };
        field[1..1 + encoded.len()].copy_from_slice(&encoded);
        let last = field.len() - 1;
        field[last] = (encoded.len() + 1) as u8;
        cut
    }

    /// Encodes a symbolic link target as path components (ECMA-167
    /// 4/14.16). `None` when a component does not fit 255 bytes.
    pub(crate) fn encode_symlink(target: &[u8]) -> Option<Vec<u8>> {
        let text = core::str::from_utf8(target).ok()?;
        let mut out = Vec::new();
        if text.starts_with('/') {
            out.extend_from_slice(&[1, 0, 0, 0]);
        }
        for part in text.split('/').filter(|part| !part.is_empty()) {
            match part {
                "." => out.extend_from_slice(&[4, 0, 0, 0]),
                ".." => out.extend_from_slice(&[3, 0, 0, 0]),
                name => {
                    let encoded = encode_cs0(name);
                    let len = u8::try_from(encoded.len()).ok()?;
                    out.extend_from_slice(&[5, len, 0, 0]);
                    out.extend_from_slice(&encoded);
                }
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cs0_round_trips_latin1_and_utf16() {
        let mut out = [0u8; 64];
        let latin = encode_cs0("caf\u{e9}");
        assert_eq!(latin, b"\x08caf\xe9");
        let len = decode_cs0(&latin, &mut out).unwrap();
        assert_eq!(&out[..len], "caf\u{e9}".as_bytes());

        let wide = encode_cs0("\u{6587}\u{1F600}");
        assert_eq!(wide[0], 16);
        let len = decode_cs0(&wide, &mut out).unwrap();
        assert_eq!(&out[..len], "\u{6587}\u{1F600}".as_bytes());

        let lone = [16, 0xD8, 0x3D, 0x00, 0x41];
        let len = decode_cs0(&lone, &mut out).unwrap();
        assert_eq!(&out[..len], "\u{FFFD}A".as_bytes());
        assert!(decode_cs0(&[7, b'a'], &mut out).is_none());
        assert!(decode_cs0(&[8, b'a', b'b'], &mut out[..1]).is_none());
    }

    #[test]
    fn names_are_sanitized() {
        let mut out = [0u8; 16];
        let len = decode_name(b"\x08a/b\0", &mut out).unwrap();
        assert_eq!(&out[..len], b"a_b_");
        let len = decode_name(b"\x08..", &mut out).unwrap();
        assert_eq!(&out[..len], b"__");
        let len = decode_name(b"\x08", &mut out).unwrap();
        assert_eq!(&out[..len], b"_");
    }

    #[test]
    fn dstrings_round_trip_and_cut_at_characters() {
        let mut field = [0u8; 8];
        assert!(!write_dstring(&mut field, "ABC"));
        let mut out = [0u8; 16];
        let len = decode_dstring(&field, &mut out);
        assert_eq!(&out[..len], b"ABC");

        assert!(write_dstring(&mut field, "\u{1F600}\u{1F600}"));
        let len = decode_dstring(&field, &mut out);
        assert_eq!(&out[..len], "\u{1F600}".as_bytes());

        for hostile in [&[][..], &[8], &[8, 0], &[8, b'a', 1], &[8, b'a', 0xFF]] {
            assert_eq!(decode_dstring(hostile, &mut out), 0);
        }
    }

    #[test]
    fn symlink_targets_round_trip() {
        let data = encode_symlink(b"/usr/../bin/./sh").unwrap();
        assert_eq!(
            data,
            [
                &[1, 0, 0, 0][..],
                &[5, 4, 0, 0, 8, b'u', b's', b'r'],
                &[3, 0, 0, 0],
                &[5, 4, 0, 0, 8, b'b', b'i', b'n'],
                &[4, 0, 0, 0],
                &[5, 3, 0, 0, 8, b's', b'h'],
            ]
            .concat()
        );
        assert!(encode_symlink(&[0xFF]).is_none());
        assert!(encode_symlink("x".repeat(300).as_bytes()).is_none());
    }
}

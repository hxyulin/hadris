//! 8.3 short names: generation from long names, case flags and name
//! validation.

/// Punctuation allowed in a short name besides letters and digits.
pub(crate) const ALLOWED_SYMBOLS: &[u8] = b"$%'-_@~`!(){}^#&";

/// `DIR_NTRes` bit: the base name is stored uppercase but displayed lowercase.
pub(crate) const LOWER_BASE: u8 = 0x08;
/// `DIR_NTRes` bit: the extension is stored uppercase but displayed lowercase.
pub(crate) const LOWER_EXT: u8 = 0x10;

/// Byte that stands for a leading `0xE5` on disk, since `0xE5` marks a
/// deleted entry.
const KANJI_LEAD: u8 = 0x05;
const DELETED: u8 = 0xE5;

/// Replaces a leading `0xE5` with `0x05` before the name is written.
pub(crate) fn to_disk(name: &mut [u8; 11]) {
    if name[0] == DELETED {
        name[0] = KANJI_LEAD;
    }
}

/// Restores a leading `0xE5` stored as `0x05`.
pub(crate) fn from_disk(name: &mut [u8; 11]) {
    if name[0] == KANJI_LEAD {
        name[0] = DELETED;
    }
}

/// Longest UTF-8 display form of a short name: 11 characters of up to 4
/// bytes and a dot.
pub(crate) const DISPLAY_MAX: usize = 11 * 4 + 1;

/// Writes the display form of a stored short name as UTF-8 and returns its
/// length: a leading `0x05` read as `0xE5`, padding dropped, the `DIR_NTRes`
/// case bits applied, and bytes above `0x7F` decoded with `decode`.
pub(crate) fn display(
    stored: &[u8; 11],
    nt_case: u8,
    decode: impl Fn(u8) -> char,
    out: &mut [u8; DISPLAY_MAX],
) -> usize {
    let mut name = *stored;
    from_disk(&mut name);
    let (base, ext) = name.split_at(8);
    let trim = |part: &[u8]| part.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    let mut len = 0;
    let mut push = |byte: u8, lower: bool| {
        let ch = if byte < 0x80 {
            let byte = if lower {
                byte.to_ascii_lowercase()
            } else {
                byte
            };
            byte as char
        } else {
            decode(byte)
        };
        len += ch.encode_utf8(&mut out[len..]).len();
    };
    for &byte in &base[..trim(base)] {
        push(byte, nt_case & LOWER_BASE != 0);
    }
    let ext = &ext[..trim(ext)];
    if !ext.is_empty() {
        push(b'.', false);
        for &byte in ext {
            push(byte, nt_case & LOWER_EXT != 0);
        }
    }
    len
}

/// Whether `name` is a valid long name: not empty, `.` or `..`, at most 255
/// UTF-16 code units, and free of control characters and `"*/:<>?\|`.
pub(crate) fn is_valid_long_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.encode_utf16().count() <= super::lfn::MAX_UNITS
        && !name.chars().any(|ch| {
            ch <= '\u{1f}' || matches!(ch, '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|')
        })
}

fn process_char(ch: char, encode: &impl Fn(char) -> Option<u8>) -> u8 {
    if ch.is_ascii_alphanumeric() {
        ch.to_ascii_uppercase() as u8
    } else if ALLOWED_SYMBOLS.contains(&(ch as u8)) {
        ch as u8
    } else if ch == ' ' || ch == '.' {
        0
    } else if ch.is_ascii() {
        b'_'
    } else {
        encode(ch).unwrap_or(b'_')
    }
}

fn hash(name: &str, suffix: u8) -> u16 {
    let mut hash = suffix as u16;
    for &byte in name.as_bytes() {
        hash = hash.wrapping_mul(37).wrapping_add(byte as u16);
    }
    hash
}

/// Generates the 11-byte short name for `name` with numeric tail `suffix`
/// (0 for none, `~1` to `~4` for 1 to 4, a hashed `~HHHH` above that).
/// Non-ASCII characters go through `encode`, the OEM code page, and become
/// `_` when it has no byte for them. `None` when nothing representable
/// remains.
pub(crate) fn generate(
    name: &str,
    suffix: u8,
    encode: impl Fn(char) -> Option<u8>,
) -> Option<[u8; 11]> {
    let (base, ext) = match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos + 1..]),
        _ => (name, ""),
    };
    let (base, ext) = if base.chars().all(|ch| ch == '.' || ch == ' ') && !ext.is_empty() {
        (ext, "")
    } else {
        (base, ext)
    };

    let mut out = [b' '; 11];
    let mut base_len = 0;
    for ch in base.chars() {
        if (base_len >= 6 && suffix > 0) || base_len >= 8 {
            break;
        }
        let byte = process_char(ch, &encode);
        if byte != 0 {
            out[base_len] = byte;
            base_len += 1;
        }
    }

    if suffix > 0 {
        let mut tail = [0u8; 5];
        let tail_len = if suffix <= 4 {
            tail[0] = b'~';
            tail[1] = b'0' + suffix;
            2
        } else {
            let hash = hash(name, suffix);
            tail[0] = b'~';
            for (i, digit) in tail[1..].iter_mut().enumerate() {
                let nibble = ((hash >> ((3 - i) * 4)) & 0xF) as u8;
                *digit = if nibble < 10 {
                    b'0' + nibble
                } else {
                    b'A' + nibble - 10
                };
            }
            5
        };
        base_len = base_len.min(8 - tail_len - if suffix <= 4 { 0 } else { 1 });
        out[base_len..base_len + tail_len].copy_from_slice(&tail[..tail_len]);
        out[base_len + tail_len..8].fill(b' ');
        base_len += tail_len;
    }

    let mut ext_len = 0;
    for ch in ext.chars() {
        if ext_len >= 3 {
            break;
        }
        let byte = process_char(ch, &encode);
        if byte != 0 {
            out[8 + ext_len] = byte;
            ext_len += 1;
        }
    }

    if base_len == 0 && ext_len == 0 {
        return None;
    }
    Some(out)
}

/// The `DIR_NTRes` case bits that let `name` be stored as a short entry
/// alone, or `None` when it needs LFN entries: too long, several dots,
/// mixed case within the base or extension, or characters outside the
/// short-name set. An uppercase 8.3 name gives `Some(0)`.
pub(crate) fn case_bits(name: &str) -> Option<u8> {
    let (base, ext) = match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos + 1..]),
        _ => (name, ""),
    };
    if base.is_empty() || base.chars().count() > 8 || ext.chars().count() > 3 {
        return None;
    }
    if name.matches('.').count() > 1 {
        return None;
    }

    fn part_is_lower(part: &str) -> Option<bool> {
        let mut seen_lower = false;
        let mut seen_upper = false;
        for c in part.chars() {
            if !c.is_ascii() {
                return None;
            }
            let upper = (c as u8).to_ascii_uppercase();
            if !(upper.is_ascii_uppercase()
                || upper.is_ascii_digit()
                || ALLOWED_SYMBOLS.contains(&upper))
            {
                return None;
            }
            seen_lower |= c.is_ascii_lowercase();
            seen_upper |= c.is_ascii_uppercase();
        }
        if seen_lower && seen_upper {
            return None;
        }
        Some(seen_lower)
    }

    let mut bits = 0;
    if part_is_lower(base)? {
        bits |= LOWER_BASE;
    }
    if part_is_lower(ext)? {
        bits |= LOWER_EXT;
    }
    Some(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii(name: &str, suffix: u8) -> Option<[u8; 11]> {
        generate(name, suffix, |_| None)
    }

    #[test]
    fn generates_basis_names() {
        assert_eq!(ascii("readme.txt", 0), Some(*b"README  TXT"));
        assert_eq!(ascii("a long name.html", 0), Some(*b"ALONGNAMHTM"));
        assert_eq!(ascii("a+b=c", 0), Some(*b"A_B_C      "));
        assert_eq!(ascii(".bashrc", 0), Some(*b"BASHRC     "));
        assert_eq!(ascii("..dots", 0), Some(*b"DOTS       "));
        assert_eq!(ascii("...", 0), None);
    }

    #[test]
    fn numeric_tails() {
        assert_eq!(ascii("long file name.txt", 1), Some(*b"LONGFI~1TXT"));
        assert_eq!(ascii("ab.txt", 4), Some(*b"AB~4    TXT"));
        let hashed = ascii("long file name.txt", 5).unwrap();
        assert_eq!(&hashed[..3], b"LO~");
        assert!(hashed[3..7].iter().all(u8::is_ascii_hexdigit));
        assert_eq!(&hashed[8..], b"TXT");
    }

    #[test]
    fn non_ascii_goes_through_code_page() {
        assert_eq!(
            generate("caf\u{e9}", 0, |c| (c == '\u{e9}').then_some(0x82)),
            Some(*b"CAF\x82       ")
        );
        assert_eq!(ascii("caf\u{e9}", 0), Some(*b"CAF_       "));
    }

    #[test]
    fn case_bits_for_short_names() {
        assert_eq!(case_bits("README.TXT"), Some(0));
        assert_eq!(case_bits("readme.txt"), Some(LOWER_BASE | LOWER_EXT));
        assert_eq!(case_bits("README.txt"), Some(LOWER_EXT));
        assert_eq!(case_bits("ReadMe.txt"), None);
        assert_eq!(case_bits("toolongname.txt"), None);
        assert_eq!(case_bits("a.b.c"), None);
        assert_eq!(case_bits("a b"), None);
    }

    #[test]
    fn kanji_lead_byte() {
        let mut name = *b"\xE5ABC       ";
        to_disk(&mut name);
        assert_eq!(name[0], 0x05);
        from_disk(&mut name);
        assert_eq!(name[0], 0xE5);
    }

    fn shown(stored: &[u8; 11], nt_case: u8) -> std::string::String {
        let mut out = [0u8; DISPLAY_MAX];
        let len = display(stored, nt_case, |_| char::REPLACEMENT_CHARACTER, &mut out);
        std::string::String::from_utf8(out[..len].to_vec()).unwrap()
    }

    #[test]
    fn display_forms() {
        assert_eq!(shown(b"README  TXT", 0), "README.TXT");
        assert_eq!(shown(b"README  TXT", LOWER_BASE), "readme.TXT");
        assert_eq!(shown(b"MAKEFILE   ", LOWER_BASE | LOWER_EXT), "makefile");
        assert_eq!(shown(b"\x05BC     TXT", 0), "\u{FFFD}BC.TXT");
        assert_eq!(
            shown(b"\xE5\xE5\xE5\xE5\xE5\xE5\xE5\xE5\xE5\xE5\xE5", 0)
                .chars()
                .count(),
            12
        );
        let mut out = [0u8; DISPLAY_MAX];
        let len = display(
            b"\x05BC     TXT",
            0,
            |b| if b == 0xE5 { '\u{3C3}' } else { '?' },
            &mut out,
        );
        assert_eq!(&out[..len], "\u{3C3}BC.TXT".as_bytes());
    }

    #[test]
    fn long_name_validation() {
        assert!(is_valid_long_name("hello world.txt"));
        assert!(!is_valid_long_name(""));
        assert!(!is_valid_long_name(".."));
        assert!(!is_valid_long_name("a:b"));
        assert!(!is_valid_long_name("tab\there"));
        assert!(is_valid_long_name(&"x".repeat(255)));
        assert!(!is_valid_long_name(&"x".repeat(256)));
    }
}

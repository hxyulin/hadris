//! 8.3 short names: generation from long names, case flags and name
//! validation.

/// Punctuation allowed in a short name besides letters and digits.
pub const ALLOWED_SYMBOLS: &[u8] = b"$%'-_@~`!(){}^#&";

use crate::dirent::{ENTRY_FREE as DELETED, ENTRY_KANJI_E5 as KANJI_LEAD};
use crate::dirent::{NT_LOWER_BASE as LOWER_BASE, NT_LOWER_EXTENSION as LOWER_EXT};

/// Replaces a leading `0xE5` with `0x05` before the name is written.
pub fn to_disk(name: &mut [u8; 11]) {
    if name[0] == DELETED {
        name[0] = KANJI_LEAD;
    }
}

/// Restores a leading `0xE5` stored as `0x05`.
pub fn from_disk(name: &mut [u8; 11]) {
    if name[0] == KANJI_LEAD {
        name[0] = DELETED;
    }
}

/// Longest UTF-8 display form of a short name: 11 characters of up to 4
/// bytes and a dot.
pub const DISPLAY_MAX: usize = 11 * 4 + 1;
/// Most characters a displayed short name has: eight, a dot and three.
pub const DISPLAY_CHARS: usize = 12;

/// Writes the display form of a stored short name as UTF-8 and returns its
/// length: a leading `0x05` read as `0xE5`, padding dropped, the `DIR_NTRes`
/// case bits applied, and bytes above `0x7F` decoded with `decode`.
pub fn display(
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

/// Writes a stored volume label as UTF-8 and returns its length: a leading
/// `0x05` read as `0xE5`, trailing spaces dropped, and bytes above `0x7F`
/// decoded with `decode`, the code page of short names. Unlike a short
/// name, a label has no dot and keeps its spaces inside.
pub fn display_label(
    stored: &[u8; 11],
    decode: impl Fn(u8) -> char,
    out: &mut [u8; DISPLAY_MAX],
) -> usize {
    let mut label = *stored;
    from_disk(&mut label);
    let end = label.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    let mut len = 0;
    for &byte in &label[..end] {
        let ch = if byte < 0x80 {
            byte as char
        } else {
            decode(byte)
        };
        len += ch.encode_utf8(&mut out[len..]).len();
    }
    len
}

/// Whether `name` is a valid long name: not empty, `.` or `..`, at most 255
/// UTF-16 code units, and free of control characters and `"*/:<>?\|`.
pub fn is_valid_long_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.encode_utf16().count() <= crate::lfn::MAX_UNITS
        && !name.chars().any(|ch| {
            ch <= '\u{1f}' || matches!(ch, '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|')
        })
}

fn process_char(ch: char, encode: &impl Fn(char) -> Option<u8>) -> u8 {
    if !ch.is_ascii() {
        return encode(ch).filter(|&byte| byte >= 0x80).unwrap_or(b'_');
    }
    let byte = ch as u8;
    if byte.is_ascii_alphanumeric() {
        byte.to_ascii_uppercase()
    } else if ALLOWED_SYMBOLS.contains(&byte) {
        byte
    } else if byte == b' ' || byte == b'.' {
        0
    } else {
        b'_'
    }
}

fn hash(name: &str, suffix: u8) -> u16 {
    let mut hash = suffix as u16;
    for &byte in name.as_bytes() {
        hash = hash.wrapping_mul(37).wrapping_add(byte as u16);
    }
    hash
}

/// Generates the 11-byte short name for `name` with numeric tail `suffix`:
/// 0 for none, `~1` to `~4` for 1 to 4. Above that, as Windows does, the
/// first two basis characters, four hex digits hashed from `name` and a
/// `~1` to `~9` tail, the hash changing every nine suffixes.
/// ASCII letters are uppercased. Non-ASCII characters go through `encode`,
/// which folds their case and maps them to the OEM code page, and become
/// `_` when it has no byte above `0x7F` for them, so no Unicode case tables
/// are linked unless `encode` uses them. `None` when nothing representable
/// remains.
pub fn generate(name: &str, suffix: u8, encode: impl Fn(char) -> Option<u8>) -> Option<[u8; 11]> {
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
        let mut tail = [0u8; 6];
        let tail_len = if suffix <= 4 {
            tail[0] = b'~';
            tail[1] = b'0' + suffix;
            2
        } else {
            let round = suffix - 5;
            let hash = hash(name, round / 9);
            for (i, digit) in tail[..4].iter_mut().enumerate() {
                let nibble = ((hash >> ((3 - i) * 4)) & 0xF) as u8;
                *digit = if nibble < 10 {
                    b'0' + nibble
                } else {
                    b'A' + nibble - 10
                };
            }
            tail[4] = b'~';
            tail[5] = b'1' + round % 9;
            6
        };
        base_len = base_len.min(8 - tail_len);
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
pub fn case_bits(name: &str) -> Option<u8> {
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
    fn labels_decode_through_the_code_page() {
        let show = |stored: &[u8; 11]| {
            let mut out = [0u8; DISPLAY_MAX];
            let len = display_label(
                stored,
                |b| {
                    if b == 0x82 {
                        'é'
                    } else {
                        char::REPLACEMENT_CHARACTER
                    }
                },
                &mut out,
            );
            std::string::String::from_utf8(out[..len].to_vec()).unwrap()
        };
        assert_eq!(show(b"MY DISK    "), "MY DISK");
        assert_eq!(show(b"CAF\x82       "), "CAFé");
        assert_eq!(show(b"\x05BC        "), "\u{FFFD}BC");
        assert_eq!(
            show(b"\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF"),
            "\u{FFFD}".repeat(11)
        );
        assert_eq!(show(b"           "), "");
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
        let hashed = |suffix| ascii("long file name.txt", suffix).unwrap();
        let first = hashed(5);
        assert_eq!(&first[..2], b"LO");
        assert!(first[2..6].iter().all(u8::is_ascii_hexdigit));
        assert_eq!(&first[6..], b"~1TXT");
        let ninth = hashed(13);
        assert_eq!(ninth[..6], first[..6]);
        assert_eq!(&ninth[6..], b"~9TXT");
        let next = hashed(14);
        assert_ne!(next[2..6], first[2..6]);
        assert_eq!(&next[6..], b"~1TXT");
        assert_eq!(&ascii("a.txt", 5).unwrap()[5..], b"~1 TXT");
        let mut all: std::vec::Vec<_> = (5..=255).map(hashed).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 251);
    }

    #[test]
    fn non_ascii_goes_through_code_page() {
        assert_eq!(
            generate("caf\u{e9}", 0, |c| {
                let upper = char::from_u32(crate::fold_unicode(c as u16) as u32).unwrap();
                (upper == '\u{c9}').then_some(0x90)
            }),
            Some(*b"CAF\x90       ")
        );
        assert_eq!(ascii("caf\u{e9}", 0), Some(*b"CAF_       "));
    }

    #[test]
    fn non_ascii_never_truncates_to_ascii() {
        assert_eq!(ascii("\u{121}a\u{12E}b", 0), Some(*b"_A_B       "));
        assert_eq!(
            generate("x\u{e9}", 0, |_| Some(b'*')),
            Some(*b"X_         ")
        );
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

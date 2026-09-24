//! Tree names converted to the identifiers of each tree.

use alloc::vec::Vec;

use crate::options::{IsoLevel, NameCase};

/// The identifier rules of one tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rules {
    /// The primary tree: Level 1 or Level 2 names.
    Primary { level: IsoLevel, case: NameCase },
    /// The enhanced tree: 207 bytes, lowercase kept.
    Enhanced,
    /// The Joliet tree: UCS-2, 64 characters.
    Joliet,
}

/// The longest Joliet identifier, in UCS-2 characters.
pub(crate) const JOLIET_MAX_CHARS: usize = 64;

/// The byte a name character becomes: ASCII letters, digits and `_`, and
/// `_` for every other character.
fn convert_char(c: char, case: NameCase) -> u8 {
    match c {
        'a'..='z' if case == NameCase::Upper => c.to_ascii_uppercase() as u8,
        _ if c.is_ascii_alphanumeric() || c == '_' => c as u8,
        _ => b'_',
    }
}

/// Appends the first `max` characters of `part`, converted.
fn push_converted(out: &mut Vec<u8>, part: &str, case: NameCase, max: usize) {
    out.extend(part.chars().take(max).map(|c| convert_char(c, case)));
}

/// A Level 1 file identifier: 8.3 with `;1`.
///
/// @hadris-spec ECMA-119:7.5.1
/// @hadris-compliance full
/// @hadris-tests iso::spec::hadris_iso_matches_ecma_119_oracle
pub(crate) fn convert_l1(name: &str, case: NameCase) -> Vec<u8> {
    let mut out = Vec::with_capacity(14);
    match name.rsplit_once('.') {
        Some((base, ext)) => {
            push_converted(&mut out, base, case, 8);
            out.push(b'.');
            push_converted(&mut out, ext, case, 3);
        }
        None => {
            push_converted(&mut out, name, case, 8);
            out.push(b'.');
        }
    }
    out.extend_from_slice(b";1");
    out
}

/// `name` split at its last dot and cut to `max` characters, dot included:
/// the base is cut first, so the extension stays unless it alone is too
/// long. With `separator`, a name without an extension still ends in `.`.
fn convert_long(name: &str, case: NameCase, max: usize, separator: bool) -> Vec<u8> {
    let mut out = Vec::new();
    match name.rsplit_once('.') {
        Some((base, ext)) => {
            let ext_len = ext.chars().count().min(max.saturating_sub(2));
            push_converted(&mut out, base, case, max - 1 - ext_len);
            out.push(b'.');
            push_converted(&mut out, ext, case, ext_len);
        }
        None => {
            push_converted(&mut out, name, case, max);
            if separator {
                out.push(b'.');
            }
        }
    }
    out
}

/// A Level 2 file identifier: 30 bytes, then `.` when the name has no
/// extension (ECMA-119 7.5.1), and `;1`.
pub(crate) fn convert_l2(name: &str, case: NameCase) -> Vec<u8> {
    let mut out = convert_long(name, case, 30, true);
    out.extend_from_slice(b";1");
    out
}

/// An enhanced tree file identifier: 207 bytes, no version.
pub(crate) fn convert_l3(name: &str) -> Vec<u8> {
    convert_long(name, NameCase::Preserve, 207, false)
}

/// Whether Joliet forbids `c` in an identifier.
fn joliet_forbidden(c: char) -> bool {
    matches!(c, '\0'..='\u{1F}' | '*' | '/' | ':' | ';' | '?' | '\\')
}

/// A big-endian UCS-2 Joliet identifier. Characters outside the BMP and
/// characters Joliet forbids become `_`; names are cut at
/// [`JOLIET_MAX_CHARS`].
pub(crate) fn convert_joliet(name: &str) -> Vec<u8> {
    name.chars()
        .take(JOLIET_MAX_CHARS)
        .flat_map(|c| {
            let unit = if (c as u32) <= 0xFFFF && !joliet_forbidden(c) {
                c as u16
            } else {
                u16::from(b'_')
            };
            unit.to_be_bytes()
        })
        .collect()
}

/// Why the Joliet identifier of `name` differs from it, if it does.
pub(crate) fn joliet_change(name: &str) -> Option<&'static str> {
    if name.chars().count() > JOLIET_MAX_CHARS {
        Some("the Joliet name is cut to 64 characters")
    } else if name.chars().any(|c| (c as u32) > 0xFFFF) {
        Some("characters outside the Basic Multilingual Plane become _ in the Joliet name")
    } else if name.chars().any(joliet_forbidden) {
        Some("characters Joliet forbids become _ in the Joliet name")
    } else {
        None
    }
}

fn primary_directory(name: &str, max: usize, case: NameCase) -> Vec<u8> {
    let mut out = Vec::with_capacity(max);
    push_converted(&mut out, name, case, max);
    out
}

impl Rules {
    /// The identifier of the file `name`.
    pub(crate) fn file(self, name: &str) -> Vec<u8> {
        match self {
            Self::Primary {
                level: IsoLevel::L1,
                case,
            } => convert_l1(name, case),
            Self::Primary { case, .. } => convert_l2(name, case),
            Self::Enhanced => convert_l3(name),
            Self::Joliet => convert_joliet(name),
        }
    }

    /// The identifier of the directory `name`.
    pub(crate) fn directory(self, name: &str) -> Vec<u8> {
        match self {
            Self::Primary {
                level: IsoLevel::L1,
                case,
            } => primary_directory(name, 8, case),
            Self::Primary { case, .. } => primary_directory(name, 31, case),
            Self::Enhanced => primary_directory(name, 31, NameCase::Preserve),
            Self::Joliet => convert_joliet(name),
        }
    }

    /// `name` with the suffix `_n` before its extension and version, cut to
    /// stay within the tree's limits.
    pub(crate) fn dedup(self, name: &[u8], n: usize) -> Vec<u8> {
        let suffix = alloc::format!("_{n}");
        match self {
            Self::Joliet => joliet_dedup(name, &suffix),
            _ => iso_dedup(name, &suffix, self),
        }
    }
}

fn joliet_dedup(name: &[u8], suffix: &str) -> Vec<u8> {
    let suffix: Vec<u8> = suffix.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let dot = name
        .chunks_exact(2)
        .rposition(|pair| pair == [0x00, b'.'])
        .map(|index| index * 2);
    let (base, ext) = match dot {
        Some(pos) => (&name[..pos], &name[pos..]),
        None => (name, &[][..]),
    };
    let max_base = 206usize.saturating_sub(ext.len() + suffix.len());
    let base = &base[..base.len().min(max_base) & !1];
    let mut out = Vec::with_capacity(base.len() + suffix.len() + ext.len());
    out.extend_from_slice(base);
    out.extend_from_slice(&suffix);
    out.extend_from_slice(ext);
    out
}

fn iso_dedup(name: &[u8], suffix: &str, rules: Rules) -> Vec<u8> {
    let (base_name, version) = match name.strip_suffix(b";1") {
        Some(base) => (base, &b";1"[..]),
        None => (name, &[][..]),
    };
    let (base, ext) = match base_name.iter().rposition(|&b| b == b'.') {
        Some(pos) => (&base_name[..pos], &base_name[pos..]),
        None => (base_name, &[][..]),
    };
    let max_total = match rules {
        Rules::Primary {
            level: IsoLevel::L1,
            ..
        } => 8,
        Rules::Primary { .. } => 30usize.saturating_sub(ext.len()),
        _ => 207usize.saturating_sub(ext.len() + version.len()),
    };
    let max_base = max_total.saturating_sub(suffix.len());
    let base = &base[..base.len().min(max_base)];
    let mut out = Vec::with_capacity(base.len() + suffix.len() + ext.len() + version.len());
    out.extend_from_slice(base);
    out.extend_from_slice(suffix.as_bytes());
    out.extend_from_slice(ext);
    out.extend_from_slice(version);
    out
}

/// Whether `stored`, the primary identifier of `name`, keeps it apart from
/// case and the version.
pub(crate) fn primary_keeps(name: &str, stored: &[u8]) -> bool {
    let stored = crate::name::strip_version(stored);
    stored.eq_ignore_ascii_case(name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const L1: Rules = Rules::Primary {
        level: IsoLevel::L1,
        case: NameCase::Upper,
    };
    const L2: Rules = Rules::Primary {
        level: IsoLevel::L2,
        case: NameCase::Upper,
    };

    #[test]
    fn level_1_names() {
        assert_eq!(
            convert_l1("this-is-the-original-file.@very-long-ext", NameCase::Upper),
            b"THIS_IS_._VE;1"
        );
        assert_eq!(
            convert_l1(
                "this-is-the-original-file.@very-long-ext",
                NameCase::Preserve
            ),
            b"this_is_._ve;1"
        );
        assert_eq!(convert_l1("file.", NameCase::Upper), b"FILE.;1");
        assert_eq!(convert_l1("..", NameCase::Upper), b"_.;1");
        assert_eq!(convert_l1("LONGFILENAME", NameCase::Upper), b"LONGFILE.;1");
        assert_eq!(convert_l1("README", NameCase::Upper), b"README.;1");
        assert_eq!(convert_l1("x.tar.gz", NameCase::Upper), b"X_TAR.GZ;1");
        assert_eq!(
            convert_l1("longname1.longext", NameCase::Upper),
            b"LONGNAME.LON;1"
        );
        assert!(convert_l1("café.txt", NameCase::Upper).len() <= 14);
    }

    #[test]
    fn level_2_and_enhanced_names() {
        assert_eq!(convert_l2("readme.txt", NameCase::Upper), b"README.TXT;1");
        assert_eq!(
            convert_l2(
                "this-is-a-very-long-directory-name-without-extension",
                NameCase::Upper
            ),
            b"THIS_IS_A_VERY_LONG_DIRECTORY_.;1"
        );
        assert_eq!(convert_l2("README", NameCase::Upper), b"README.;1");
        assert_eq!(convert_l2("x.tar.gz", NameCase::Upper), b"X_TAR.GZ;1");
        assert_eq!(convert_l3("x.tar.gz"), b"x_tar.gz");
        assert_eq!(convert_l3("readme.txt"), b"readme.txt");
        assert_eq!(convert_l3(&"a".repeat(250)).len(), 207);
    }

    #[test]
    fn names_map_per_character() {
        assert_eq!(convert_l1("caf\u{e9}.txt", NameCase::Upper), b"CAF_.TXT;1");
        assert_eq!(
            convert_l1(&"\u{e9}".repeat(9), NameCase::Upper),
            b"________.;1"
        );
        assert_eq!(
            convert_l2("r\u{e9}sum\u{e9}.pdf", NameCase::Upper),
            b"R_SUM_.PDF;1"
        );
        assert_eq!(convert_l3("\u{1F600}.txt"), b"_.txt");
        assert_eq!(L2.directory("\u{f1}and\u{fa}"), b"_AND_");
        assert_eq!(L1.directory(&"\u{e9}".repeat(9)), b"________");
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = alloc::format!("{}.txt", "a".repeat(40));
        let mut want = "A".repeat(26).into_bytes();
        want.extend_from_slice(b".TXT;1");
        assert_eq!(convert_l2(&long, NameCase::Upper), want);
        let long = alloc::format!("{}.txt", "a".repeat(250));
        let l3 = convert_l3(&long);
        assert_eq!(l3.len(), 207);
        assert!(l3.ends_with(b".txt"));
        let ext = alloc::format!("a.{}", "b".repeat(40));
        let l2 = convert_l2(&ext, NameCase::Upper);
        assert_eq!(l2.len(), 32);
        assert!(l2.starts_with(b"A."));
    }

    #[test]
    fn joliet_names() {
        assert_eq!(convert_joliet(&"a".repeat(200)).len(), 128);
        assert_eq!(convert_joliet("a\u{1F600}b"), [0, b'a', 0, b'_', 0, b'b']);
        assert_eq!(convert_joliet("a*:?"), [0, b'a', 0, b'_', 0, b'_', 0, b'_']);
        assert_eq!(
            convert_joliet("x;1\\"),
            [0, b'x', 0, b'_', 0, b'1', 0, b'_']
        );
        assert!(joliet_change(&"a".repeat(65)).is_some());
        assert!(joliet_change("a\u{1F600}").is_some());
        assert!(joliet_change("a:b").is_some());
        assert_eq!(joliet_change("caf\u{e9}.txt"), None);
    }

    #[test]
    fn dedup_suffixes() {
        assert_eq!(L1.dedup(b"README.TXT;1", 1), b"README_1.TXT;1");
        assert_eq!(L1.dedup(b"FILENAME.;1", 1), b"FILENA_1.;1");
        assert_eq!(L2.dedup(b"LONGFILENAME.EXT;1", 2), b"LONGFILENAME_2.EXT;1");
        assert_eq!(Rules::Enhanced.dedup(b"README.TXT", 1), b"README_1.TXT");
    }
}

//! FAT name handling shared by `FatFs` and the embedded `Fat`: matching a
//! query against an entry, planning a new name's short name and long-name
//! slots, and filling a short entry's times and attributes.

use hadris_fat_raw::lfn::{self, Encoded};
use hadris_fat_raw::{self as raw, ShortEntry, date, name as names, short_name};
use hadris_fs::{Attributes, CodePage, DateTime, ErrorKind, Metadata, Permissions};

/// Short-name candidates a directory scan checks at once.
pub(crate) const CANDIDATES: usize = 6;

/// The attribute bits [`Attributes`] maps to.
pub(crate) const ATTR_MAPPED: [(u8, Attributes); 4] = [
    (raw::ATTR_READ_ONLY, Attributes::READ_ONLY),
    (raw::ATTR_HIDDEN, Attributes::HIDDEN),
    (raw::ATTR_SYSTEM, Attributes::SYSTEM),
    (raw::ATTR_ARCHIVE, Attributes::ARCHIVE),
];

/// `ch` folded by `fold` when it is in the Basic Multilingual Plane.
pub(crate) fn fold_char(ch: char, fold: fn(u16) -> u16) -> char {
    match u16::try_from(ch as u32) {
        Ok(unit) => char::from_u32(fold(unit) as u32).unwrap_or(ch),
        Err(_) => ch,
    }
}

/// A name about to be written: its long-name entries and short-name
/// candidates.
pub(crate) struct NewName<'a> {
    pub(crate) encoded: Encoded<'a>,
    /// Candidates in on-disk form for tails none, `~1` to `~4`, and the
    /// first hashed tail, so one directory scan checks them all.
    pub(crate) candidates: [[u8; 11]; CANDIDATES],
    /// The name is its own short name up to case.
    pub(crate) lossless: bool,
    /// Set when the name can be stored as a short entry alone with these
    /// `DIR_NTRes` case bits.
    pub(crate) case_bits: Option<u8>,
}

impl<'a> NewName<'a> {
    /// Checks `text` and generates its short-name candidates, folding the
    /// case of non-ASCII characters with `fold` before `code_page` maps
    /// them.
    pub(crate) fn new(
        text: &'a str,
        code_page: &dyn CodePage,
        fold: fn(u16) -> u16,
    ) -> Result<Self, ErrorKind> {
        if text.encode_utf16().count() > lfn::MAX_UNITS {
            return Err(ErrorKind::NameTooLong);
        }
        if !short_name::is_valid_long_name(text) || text.ends_with(['.', ' ']) {
            return Err(ErrorKind::InvalidInput);
        }
        let encoded = Encoded::new(text).ok_or(ErrorKind::InvalidInput)?;
        let mut candidates = [[0u8; 11]; CANDIDATES];
        for (suffix, candidate) in candidates.iter_mut().enumerate() {
            if let Some(mut name) = short_name(text, suffix as u8, code_page, fold) {
                short_name::to_disk(&mut name);
                *candidate = name;
            }
        }
        let mut shown = [0u8; short_name::DISPLAY_MAX];
        let len = short_name::display(&candidates[0], 0, |byte| code_page.decode(byte), &mut shown);
        let lossless = candidates[0][0] != 0
            && text.is_ascii()
            && text
                .bytes()
                .map(|b| b.to_ascii_uppercase())
                .eq(shown[..len].iter().copied());
        Ok(Self {
            encoded,
            candidates,
            lossless,
            case_bits: short_name::case_bits(text),
        })
    }

    pub(crate) fn short_only(&self) -> bool {
        self.lossless && self.case_bits.is_some()
    }

    /// Directory slots the name needs.
    pub(crate) fn slots(&self) -> u32 {
        if self.short_only() {
            1
        } else {
            self.encoded.entries() as u32 + 1
        }
    }
}

/// The short name of `text` with tail `suffix`, in the form
/// [`short_name::generate`] returns.
pub(crate) fn short_name(
    text: &str,
    suffix: u8,
    code_page: &dyn CodePage,
    fold: fn(u16) -> u16,
) -> Option<[u8; 11]> {
    short_name::generate(text, suffix, |ch| code_page.encode(fold_char(ch, fold)))
}

#[cfg(feature = "alloc")]
pub(crate) struct Query {
    units: [u16; 24],
    overflow: Option<alloc::boxed::Box<[u16]>>,
    len: usize,
    short: bool,
    ascii: Option<[u16; 11]>,
}

#[cfg(feature = "alloc")]
impl Query {
    pub(crate) fn new(text: &str, fold: fn(u16) -> u16) -> Self {
        let mut query = Self {
            units: [0; 24],
            overflow: None,
            len: text.encode_utf16().count(),
            short: text.chars().nth(short_name::DISPLAY_CHARS).is_none(),
            ascii: None,
        };
        if query.len > lfn::MAX_UNITS {
            return query;
        }
        if query.len <= query.units.len() {
            for (out, unit) in query.units.iter_mut().zip(text.encode_utf16()) {
                *out = fold(unit);
            }
        } else {
            query.overflow = Some(
                text.encode_utf16()
                    .map(fold)
                    .collect::<alloc::vec::Vec<_>>()
                    .into_boxed_slice(),
            );
        }
        if text.is_ascii() {
            let (base, ext) = text.rsplit_once('.').unwrap_or((text, ""));
            if !base.is_empty()
                && base.len() <= 8
                && ext.len() <= 3
                && !base.contains('.')
                && !base.contains(' ')
                && !ext.contains(' ')
                && !text.ends_with('.')
            {
                let mut name = [fold(b' ' as u16); 11];
                for (out, byte) in name[..8].iter_mut().zip(base.bytes()) {
                    *out = fold(byte as u16);
                }
                for (out, byte) in name[8..].iter_mut().zip(ext.bytes()) {
                    *out = fold(byte as u16);
                }
                query.ascii = Some(name);
            }
        }
        query
    }

    pub(crate) fn fingerprint(&self) -> Option<u64> {
        self.overflow
            .as_deref()
            .or_else(|| self.units.get(..self.len))
            .map(|units| name_hash(units.iter().copied()))
    }

    pub(crate) fn matches(
        &self,
        long: Option<&[u16]>,
        entry: &ShortEntry,
        code_page: &dyn CodePage,
        fold: fn(u16) -> u16,
    ) -> bool {
        let Some(query) = self
            .overflow
            .as_deref()
            .or_else(|| self.units.get(..self.len))
        else {
            return false;
        };
        if long.is_some_and(|units| query.iter().copied().eq(units.iter().copied().map(fold))) {
            return true;
        }
        if !self.short {
            return false;
        }
        let stored = entry.name();
        if let Some(ascii) = self.ascii
            && stored[0] != 0x05
            && stored.is_ascii()
        {
            return stored
                .iter()
                .enumerate()
                .zip(ascii)
                .all(|((i, &byte), wanted)| {
                    let lower = entry.nt_case()
                        & if i < 8 {
                            raw::NT_LOWER_BASE
                        } else {
                            raw::NT_LOWER_EXTENSION
                        }
                        != 0;
                    let byte = if lower {
                        byte.to_ascii_lowercase()
                    } else {
                        byte
                    };
                    fold(byte as u16) == wanted
                });
        }
        let mut short = [0u8; short_name::DISPLAY_MAX];
        let len = short_name::display(
            &entry.name(),
            entry.nt_case(),
            |byte| code_page.decode(byte),
            &mut short,
        );
        core::str::from_utf8(&short[..len])
            .is_ok_and(|short| query.iter().copied().eq(short.encode_utf16().map(fold)))
    }
}

#[cfg(feature = "alloc")]
pub(crate) fn name_hash(units: impl Iterator<Item = u16>) -> u64 {
    units.fold(0xcbf29ce484222325, |hash, unit| {
        (hash ^ unit as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(feature = "alloc")]
pub(crate) fn short_fingerprint(entry: &ShortEntry, code_page: &dyn CodePage) -> u64 {
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name(),
        entry.nt_case(),
        |byte| code_page.decode(byte),
        &mut short,
    );
    name_hash(
        core::str::from_utf8(&short[..len])
            .unwrap_or("")
            .encode_utf16()
            .map(raw::fold_unicode),
    )
}

/// Whether `query` names the entry, by its long name or its short name,
/// with case folded by `fold`.
#[cfg(any(feature = "sync", feature = "async"))]
pub(crate) fn matches(
    query: &str,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &dyn CodePage,
    fold: fn(u16) -> u16,
) -> bool {
    if long.is_some_and(|units| names::eq_folded(query.encode_utf16(), units.iter().copied(), fold))
    {
        return true;
    }
    if query.chars().nth(short_name::DISPLAY_CHARS).is_some() {
        return false;
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name(),
        entry.nt_case(),
        |byte| code_page.decode(byte),
        &mut short,
    );
    core::str::from_utf8(&short[..len])
        .is_ok_and(|short| names::eq_folded(query.encode_utf16(), short.encode_utf16(), fold))
}

/// Whether `query` is exactly the entry's name.
pub(crate) fn is_exact(
    query: &str,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &dyn CodePage,
) -> bool {
    if let Some(units) = long {
        return names::utf16_chars(units.iter().copied()).eq(query.chars());
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name(),
        entry.nt_case(),
        |byte| code_page.decode(byte),
        &mut short,
    );
    short[..len] == *query.as_bytes()
}

/// Sets the entry's creation time and the modification and access times.
pub(crate) fn stamp(
    entry: &mut ShortEntry,
    created: DateTime,
    modified: DateTime,
    accessed: DateTime,
    zone: Option<i16>,
) {
    let (date, time, tenths) = date::encode(created, zone);
    entry.set_created(date, time, tenths);
    let (date, time, _) = date::encode(modified, zone);
    entry.set_modified(date, time);
    entry.set_accessed_date(date::encode(accessed, zone).0);
}

pub(crate) fn set_read_only(entry: &mut ShortEntry, read_only: bool) {
    match read_only {
        true => entry.set_attributes(entry.attributes() | raw::ATTR_READ_ONLY),
        false => entry.set_attributes(entry.attributes() & !raw::ATTR_READ_ONLY),
    }
}

pub(crate) fn apply_attributes(entry: &mut ShortEntry, attributes: Attributes) {
    for (bit, flag) in ATTR_MAPPED {
        if attributes.contains(flag) {
            entry.set_attributes(entry.attributes() | bit);
        } else {
            entry.set_attributes(entry.attributes() & !bit);
        }
    }
}

/// The attributes of `entry` that [`Attributes`] maps.
pub(crate) fn attributes(entry: &ShortEntry) -> Attributes {
    let mut attributes = Attributes::empty();
    for (bit, flag) in ATTR_MAPPED {
        if entry.attributes() & bit != 0 {
            attributes |= flag;
        }
    }
    attributes
}

/// The permissions FAT and exFAT derive: `rwx` for directories, `rw` for
/// files, and no write bits when the entry is read-only.
pub(crate) fn permissions(dir: bool, read_only: bool) -> Permissions {
    let mode = if dir { 0o755 } else { 0o644 };
    Permissions::new(if read_only { mode & !0o222 } else { mode })
}

/// The read-only bit `wanted` stands for: `None` when the volume cannot
/// report those permissions for a node of this kind.
pub(crate) fn read_only_bit(dir: bool, wanted: Permissions) -> Option<bool> {
    let read_only = wanted.bits() & 0o222 == 0;
    (permissions(dir, read_only) == wanted).then_some(read_only)
}

/// The metadata a short entry stores, with `len` as the file's length and
/// times read in the zone `zone`.
pub(crate) fn metadata(entry: &ShortEntry, dir: bool, len: u64, zone: Option<i16>) -> Metadata {
    let (created_date, created_time, created_tenths) = entry.created();
    let (modified_date, modified_time) = entry.modified();
    let attributes = attributes(entry);
    let (file_type, len) = if dir {
        (hadris_fs::FileType::Dir, 0)
    } else {
        (hadris_fs::FileType::File, len)
    };
    let mut meta = Metadata::new(
        file_type,
        permissions(dir, attributes.contains(Attributes::READ_ONLY)),
    )
    .with_len(len)
    .with_attributes(attributes);
    if let Some(time) = date::decode(created_date, created_time, created_tenths, zone) {
        meta = meta.with_created(time);
    }
    if let Some(time) = date::decode(modified_date, modified_time, 0, zone) {
        meta = meta.with_modified(time);
    }
    if let Some(time) = date::decode(entry.accessed_date(), 0, 0, zone) {
        meta = meta.with_accessed(time);
    }
    meta
}

#[cfg(all(test, feature = "alloc"))]
mod query_tests {
    use super::*;
    use hadris_fs::{Ascii, Cp437};

    #[test]
    fn prepared_queries_preserve_long_names_aliases_and_code_pages() {
        let entry = ShortEntry::new(*b"UNICO~1 TXT", 0);
        let long: alloc::vec::Vec<_> = "Ünicödé 😀.txt".encode_utf16().collect();
        assert!(Query::new("ünicödé 😀.TXT", raw::fold_unicode).matches(
            Some(&long),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(Query::new("unico~1.txt", raw::fold_unicode).matches(
            Some(&long),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("Ünicödé 😁.txt", raw::fold_unicode).matches(
            Some(&long),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("Ünicödé 😀.txt", raw::fold_unicode).matches(
            Some(&[0xd800]),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        let mut ascii = ShortEntry::new(*b"MAXALIASBIN", 0);
        ascii.set_nt_case(raw::NT_LOWER_BASE | raw::NT_LOWER_EXTENSION);
        assert!(Query::new("maxalias.bin", raw::fold_unicode).matches(
            None,
            &ascii,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("maxalias.bin.", raw::fold_unicode).matches(
            None,
            &ascii,
            &Cp437,
            raw::fold_unicode
        ));
        let no_ext = ShortEntry::new(*b"KERNEL     ", 0);
        assert!(Query::new("kernel", raw::fold_unicode).matches(
            None,
            &no_ext,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("kernel.", raw::fold_unicode).matches(
            None,
            &no_ext,
            &Cp437,
            raw::fold_unicode
        ));
        let entry = ShortEntry::new(*b"\x05ABC    TXT", 0);
        assert!(Query::new("σabc.txt", raw::fold_unicode).matches(
            None,
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("σabc.txt", raw::fold_unicode).matches(
            None,
            &entry,
            &Ascii,
            raw::fold_unicode
        ));
        assert!(Query::new("\u{f7e5}abc.txt", raw::fold_unicode).matches(
            None,
            &entry,
            &Ascii,
            raw::fold_unicode
        ));
    }

    #[test]
    fn prepared_queries_handle_the_utf16_limit() {
        let entry = ShortEntry::new(*b"SHORT   TXT", 0);
        let max = alloc::string::String::from("😀") + &"x".repeat(253);
        let units: alloc::vec::Vec<_> = max.encode_utf16().collect();
        assert_eq!(units.len(), lfn::MAX_UNITS);
        assert!(Query::new(&max, raw::fold_unicode).matches(
            Some(&units),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        let over = max + "x";
        assert!(!Query::new(&over, raw::fold_unicode).matches(
            Some(&units),
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
        assert!(!Query::new("short.txt.extra", raw::fold_unicode).matches(
            None,
            &entry,
            &Cp437,
            raw::fold_unicode
        ));
    }
}

#[cfg(all(test, feature = "alloc"))]
mod index_name_tests {
    use super::*;
    use hadris_fs::{Ascii, Cp437};

    struct HighOnly;
    impl CodePage for HighOnly {
        fn decode(&self, byte: u8) -> char {
            assert!(byte >= 0x80);
            'é'
        }
        fn encode(&self, _: char) -> Option<u8> {
            None
        }
    }

    #[test]
    fn short_index_hash_matches_ascii_case_and_code_page_queries() {
        let mut entry = ShortEntry::new(*b"LOWER   TXT", raw::ATTR_ARCHIVE);
        entry.set_nt_case(raw::NT_LOWER_BASE | raw::NT_LOWER_EXTENSION);
        for query in ["lower.txt", "LOWER.TXT"] {
            assert_eq!(
                Query::new(query, raw::fold_unicode).fingerprint(),
                Some(short_fingerprint(&entry, &HighOnly))
            );
        }
        for (name, query, page) in [
            (*b"\x82AB     TXT", "éab.txt", &Cp437 as &dyn CodePage),
            (
                *b"\x82AB     TXT",
                "\u{f782}ab.txt",
                &Ascii as &dyn CodePage,
            ),
            (*b"\x05AB     TXT", "σab.txt", &Cp437 as &dyn CodePage),
        ] {
            let entry = ShortEntry::new(name, raw::ATTR_ARCHIVE);
            let query = Query::new(query, raw::fold_unicode);
            assert!(query.matches(None, &entry, page, raw::fold_unicode));
            assert_eq!(query.fingerprint(), Some(short_fingerprint(&entry, page)));
        }
    }
}

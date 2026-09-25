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

/// Whether `query` names the entry, by its long name or its short name,
/// with case folded by `fold`.
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

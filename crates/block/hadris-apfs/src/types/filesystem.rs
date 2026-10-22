//! APFS filesystem-tree record parsing.

use crate::types::le_u64;
#[cfg(feature = "alloc")]
use alloc::borrow::Cow;
#[cfg(feature = "alloc")]
use icu_casemap::CaseMapperBorrowed;
#[cfg(feature = "alloc")]
use icu_normalizer::DecomposingNormalizerBorrowed;

/// Root directory inode number.
pub const INODE_ROOT_DIRECTORY: u64 = 2;
/// Filesystem record type for inode records.
pub const FS_TYPE_INODE: u8 = 3;
/// Filesystem record type for extended attribute records.
pub const FS_TYPE_XATTR: u8 = 4;
/// Filesystem record type for file extent records.
pub const FS_TYPE_FILE_EXTENT: u8 = 8;
/// Filesystem record type for directory entries.
pub const FS_TYPE_DIRECTORY_RECORD: u8 = 9;

/// Bits of a directory entry's flags that hold its file type.
pub const DREC_TYPE_MASK: u16 = 0x000f;
/// Directory entry file type for a directory.
pub const DT_DIR: u16 = 4;
/// Directory entry file type for a regular file.
pub const DT_REG: u16 = 8;
/// Directory entry file type for a symbolic link.
pub const DT_LNK: u16 = 10;

/// Common filesystem-tree key header (`j_key_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileSystemKey {
    /// Object/inode identifier.
    pub id: u64,
    /// APFS filesystem record type.
    pub record_type: u8,
}

impl FileSystemKey {
    /// Parses a filesystem key header.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let raw = le_u64(data, 0)?;
        Ok(Self {
            id: raw & 0x0fff_ffff_ffff_ffff,
            record_type: ((raw >> 60) & 0xf) as u8,
        })
    }
}

/// Layout of a directory entry key, chosen by the volume's incompatible
/// features; see [`crate::types::VolumeSuperblock::directory_key_layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectoryKeyLayout {
    /// `j_drec_hashed_key_t`: a 32-bit field with a 10-bit name length and a
    /// 22-bit name hash at offset 8, then the name.
    Hashed,
    /// `j_drec_key_t`: a 16-bit name length at offset 8, then the name.
    Plain,
}

/// Parsed directory entry record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DirectoryEntryRecord<'a> {
    /// Parent directory inode identifier.
    pub parent_id: u64,
    /// Child inode identifier.
    pub file_id: u64,
    /// Directory entry flags; the bits in [`DREC_TYPE_MASK`] are the file type.
    pub flags: u16,
    /// Entry name bytes as stored, without the terminating NUL. APFS names
    /// are UTF-8, but the bytes are not validated; see [`Self::name_str`].
    pub name: &'a [u8],
}

impl<'a> DirectoryEntryRecord<'a> {
    /// Parses a directory entry key/value pair stored in `layout`.
    pub fn parse(
        key: &'a [u8],
        value: &'a [u8],
        layout: DirectoryKeyLayout,
    ) -> crate::Result<Self> {
        let header = FileSystemKey::parse(key)?;
        if header.record_type != FS_TYPE_DIRECTORY_RECORD {
            return Err(crate::ApfsError::InvalidValue("directory record key type"));
        }
        let (name_len, name_start) = match layout {
            DirectoryKeyLayout::Hashed => {
                let name_len_and_hash = u32::from_le_bytes(crate::types::take(key, 8)?);
                ((name_len_and_hash & 0x3ff) as usize, 12)
            }
            DirectoryKeyLayout::Plain => {
                (u16::from_le_bytes(crate::types::take(key, 8)?) as usize, 10)
            }
        };
        let name_bytes = key
            .get(name_start..name_start + name_len)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        let name_bytes = name_bytes.strip_suffix(&[0]).unwrap_or(name_bytes);
        Ok(Self {
            parent_id: header.id,
            file_id: le_u64(value, 0)?,
            flags: u16::from_le_bytes(crate::types::take(value, 16)?),
            name: name_bytes,
        })
    }

    /// Returns the name as UTF-8.
    pub fn name_str(&self) -> crate::Result<&'a str> {
        core::str::from_utf8(self.name)
            .map_err(|_| crate::ApfsError::InvalidValue("directory name UTF-8"))
    }

    /// Returns the file type, such as [`DT_DIR`], [`DT_REG`] or [`DT_LNK`].
    pub const fn file_type(&self) -> u16 {
        self.flags & DREC_TYPE_MASK
    }
}

/// Owned parsed directory entry record.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OwnedDirectoryEntryRecord {
    /// Parent directory inode identifier.
    pub parent_id: u64,
    /// Child inode identifier.
    pub file_id: u64,
    /// Directory entry flags; the bits in [`DREC_TYPE_MASK`] are the file type.
    pub flags: u16,
    /// Entry name.
    pub name: alloc::string::String,
}

#[cfg(feature = "alloc")]
impl OwnedDirectoryEntryRecord {
    /// Parses a directory entry key/value pair stored in `layout` into an
    /// owned record. Names that are not UTF-8 fail with
    /// [`crate::ApfsError::InvalidValue`].
    pub fn parse(key: &[u8], value: &[u8], layout: DirectoryKeyLayout) -> crate::Result<Self> {
        let entry = DirectoryEntryRecord::parse(key, value, layout)?;
        Ok(Self {
            parent_id: entry.parent_id,
            file_id: entry.file_id,
            flags: entry.flags,
            name: entry.name_str()?.into(),
        })
    }

    /// Returns the file type, such as [`DT_DIR`], [`DT_REG`] or [`DT_LNK`].
    pub const fn file_type(&self) -> u16 {
        self.flags & DREC_TYPE_MASK
    }
}

/// Small parsed inode summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct InodeRecord {
    /// Inode identifier.
    pub id: u64,
    /// Parent inode identifier.
    pub parent_id: u64,
    /// Private data-stream identifier.
    pub private_id: u64,
    /// Inode mode bits.
    pub mode: u16,
    /// BSD flags (`chflags`), such as [`UF_COMPRESSED`].
    pub bsd_flags: u32,
    /// Owning user identifier.
    pub owner: u32,
    /// Owning group identifier.
    pub group: u32,
    /// Uncompressed file size when recorded by APFS.
    pub uncompressed_size: u64,
    /// Link count or directory child count.
    pub link_or_child_count: i32,
    /// Creation time as nanoseconds since the Unix epoch.
    pub create_time_ns: u64,
    /// Last content modification time as nanoseconds since the Unix epoch.
    pub modification_time_ns: u64,
    /// Last attribute-change time as nanoseconds since the Unix epoch.
    pub change_time_ns: u64,
    /// Last access time as nanoseconds since the Unix epoch.
    pub access_time_ns: u64,
    /// Exact data-stream size in bytes from the inode's `INO_EXT_TYPE_DSTREAM`
    /// extended field, when present.
    pub data_stream_size: Option<u64>,
}

impl InodeRecord {
    /// Parses an inode key/value pair.
    pub fn parse(key: &[u8], value: &[u8]) -> crate::Result<Self> {
        let header = FileSystemKey::parse(key)?;
        if header.record_type != FS_TYPE_INODE {
            return Err(crate::ApfsError::InvalidValue("inode record key type"));
        }
        Ok(Self {
            id: header.id,
            parent_id: le_u64(value, 0)?,
            private_id: le_u64(value, 8)?,
            link_or_child_count: i32::from_le_bytes(crate::types::take(value, 56)?),
            create_time_ns: le_u64(value, 16)?,
            modification_time_ns: le_u64(value, 24)?,
            change_time_ns: le_u64(value, 32)?,
            access_time_ns: le_u64(value, 40)?,
            bsd_flags: crate::types::le_u32(value, 68)?,
            owner: crate::types::le_u32(value, 72)?,
            group: crate::types::le_u32(value, 76)?,
            mode: u16::from_le_bytes(crate::types::take(value, 80)?),
            uncompressed_size: le_u64(value, 84)?,
            data_stream_size: parse_inode_data_stream_size(value)?,
        })
    }

    /// Returns whether the file data is stored compressed (`decmpfs`), which
    /// this reader does not decode.
    pub const fn is_compressed(&self) -> bool {
        self.bsd_flags & UF_COMPRESSED != 0
    }
}

/// BSD flag marking a file whose data is stored compressed.
pub const UF_COMPRESSED: u32 = 0x20;

/// Extended field type for an inode's data-stream (`INO_EXT_TYPE_DSTREAM`).
const INODE_EXT_TYPE_DATA_STREAM: u8 = 8;
/// Fixed-size prefix of `j_inode_val_t` before the extended-fields blob.
const INODE_FIXED_VALUE_SIZE: usize = 92;

/// Parses the `size` field of an inode's `j_dstream_t` extended field, if
/// the inode records one. Returns `None` when the inode has no extended
/// fields, and fails when the field table or the data a field points at is
/// truncated, so a damaged inode is reported rather than sized by its
/// extents.
fn parse_inode_data_stream_size(value: &[u8]) -> crate::Result<Option<u64>> {
    let Some(blob) = value
        .get(INODE_FIXED_VALUE_SIZE..)
        .filter(|blob| !blob.is_empty())
    else {
        return Ok(None);
    };
    let count = usize::from(u16::from_le_bytes(crate::types::take(blob, 0)?));
    let fields = blob.get(4..).ok_or(crate::ApfsError::InputTooSmall)?;
    let mut field_offset = 0usize;
    let mut data_offset = count
        .checked_mul(4)
        .ok_or(crate::ApfsError::AddressOverflow)?;
    for _ in 0..count {
        let entry = fields
            .get(field_offset..field_offset + 4)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        let field_type = entry[0];
        let size_bytes = usize::from(u16::from_le_bytes([entry[2], entry[3]]));
        field_offset += 4;
        let end = data_offset
            .checked_add(size_bytes)
            .ok_or(crate::ApfsError::AddressOverflow)?;
        let data = fields
            .get(data_offset..end)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        if field_type == INODE_EXT_TYPE_DATA_STREAM {
            return Ok(Some(le_u64(data, 0)?));
        }
        data_offset = end;
        let remainder = size_bytes % 8;
        if remainder != 0 {
            data_offset += 8 - remainder;
        }
    }
    Ok(None)
}

/// Parsed file extent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileExtentRecord {
    /// Owning filesystem object identifier.
    pub id: u64,
    /// Logical byte offset in the file.
    pub logical_address: u64,
    /// Extent length in bytes.
    pub length: u64,
    /// Extent flags from the high byte of the length field.
    pub flags: u8,
    /// Starting physical APFS block.
    pub physical_block: u64,
    /// Cryptography identifier/tweak.
    pub cryptography_id: u64,
}

impl FileExtentRecord {
    /// Parses a file extent key/value pair.
    pub fn parse(key: &[u8], value: &[u8]) -> crate::Result<Self> {
        let header = FileSystemKey::parse(key)?;
        if header.record_type != FS_TYPE_FILE_EXTENT {
            return Err(crate::ApfsError::InvalidValue("file extent key type"));
        }
        let length_and_flags = le_u64(value, 0)?;
        Ok(Self {
            id: header.id,
            logical_address: le_u64(key, 8)?,
            length: length_and_flags & 0x00ff_ffff_ffff_ffff,
            flags: ((length_and_flags >> 56) & 0xff) as u8,
            physical_block: le_u64(value, 8)?,
            cryptography_id: le_u64(value, 16)?,
        })
    }
}

impl FileExtentRecord {
    /// Parses a sealed volume's file extent tree entry (`fext_tree_key_t`
    /// and `fext_tree_val_t`). Such extents are never encrypted.
    pub fn parse_fext(key: &[u8], value: &[u8]) -> crate::Result<Self> {
        let length_and_flags = le_u64(value, 0)?;
        Ok(Self {
            id: le_u64(key, 0)?,
            logical_address: le_u64(key, 8)?,
            length: length_and_flags & 0x00ff_ffff_ffff_ffff,
            flags: ((length_and_flags >> 56) & 0xff) as u8,
            physical_block: le_u64(value, 8)?,
            cryptography_id: 0,
        })
    }
}

/// Returns a file's size: the inode's data-stream size when present, then
/// its uncompressed size when set, otherwise the end of its last extent.
pub fn stream_size(inode: &InodeRecord, extents: &[FileExtentRecord]) -> u64 {
    if let Some(size) = inode.data_stream_size {
        return size;
    }
    if inode.uncompressed_size != 0 {
        return inode.uncompressed_size;
    }
    extents
        .iter()
        .map(|extent| extent.logical_address.saturating_add(extent.length))
        .max()
        .unwrap_or(0)
}

/// Name of the extended attribute that holds a symlink's target.
pub const SYMLINK_XATTR_NAME: &str = "com.apple.fs.symlink";
/// Extended attribute flag: the data is stored inline in the record.
pub const XATTR_DATA_EMBEDDED: u16 = 0x2;

/// Parsed extended attribute record (`j_xattr_key_t` and `j_xattr_val_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct XattrRecord<'a> {
    /// Owning inode identifier.
    pub id: u64,
    /// Attribute name.
    pub name: &'a str,
    /// Attribute flags, such as [`XATTR_DATA_EMBEDDED`].
    pub flags: u16,
    /// Inline data; empty when the data is kept in a separate stream.
    pub data: &'a [u8],
}

impl<'a> XattrRecord<'a> {
    /// Parses an extended attribute key/value pair.
    pub fn parse(key: &'a [u8], value: &'a [u8]) -> crate::Result<Self> {
        let header = FileSystemKey::parse(key)?;
        if header.record_type != FS_TYPE_XATTR {
            return Err(crate::ApfsError::InvalidValue(
                "extended attribute key type",
            ));
        }
        let name_len = u16::from_le_bytes(crate::types::take(key, 8)?) as usize;
        let name_bytes = key
            .get(10..10 + name_len)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        let name_bytes = name_bytes.strip_suffix(&[0]).unwrap_or(name_bytes);
        let flags = u16::from_le_bytes(crate::types::take(value, 0)?);
        let data_len = u16::from_le_bytes(crate::types::take(value, 2)?) as usize;
        let data = if flags & XATTR_DATA_EMBEDDED != 0 {
            value
                .get(4..4 + data_len)
                .ok_or(crate::ApfsError::InputTooSmall)?
        } else {
            &[]
        };
        Ok(Self {
            id: header.id,
            name: core::str::from_utf8(name_bytes)
                .map_err(|_| crate::ApfsError::InvalidValue("extended attribute name UTF-8"))?,
            flags,
            data,
        })
    }
}

/// How a volume compares directory entry names, from its
/// `APFS_INCOMPAT_CASE_INSENSITIVE` and
/// `APFS_INCOMPAT_NORMALIZATION_INSENSITIVE` flags; see
/// [`crate::types::VolumeSuperblock::name_comparison`].
///
/// Comparison uses the Unicode default case folding and canonical
/// decomposition of Unicode Standard section 3.13, "Default Case
/// Algorithms". Apple's own tables differ from the standard for a few
/// ranges, so names that differ only there may still not match.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameComparison {
    case_insensitive: bool,
    normalization_insensitive: bool,
}

#[cfg(feature = "alloc")]
impl NameComparison {
    /// Comparison for a volume whose incompatible features set
    /// `case_insensitive` and `normalization_insensitive`.
    pub const fn new(case_insensitive: bool, normalization_insensitive: bool) -> Self {
        Self {
            case_insensitive,
            normalization_insensitive,
        }
    }

    /// Returns whether `stored` names the same object as `wanted`.
    pub fn matches(self, stored: &str, wanted: &str) -> bool {
        if stored == wanted || (!self.case_insensitive && !self.normalization_insensitive) {
            return stored == wanted;
        }
        self.canonical(stored) == self.canonical(wanted)
    }

    /// Returns `text` in the form this volume compares names in. Text that
    /// already has that form is borrowed, so a comparison that needs no
    /// folding or decomposition allocates nothing.
    fn canonical(self, text: &str) -> Cow<'_, str> {
        match (self.case_insensitive, self.normalization_insensitive) {
            (false, false) => Cow::Borrowed(text),
            (false, true) => decompose(text),
            (true, false) => fold(text),
            (true, true) => {
                // Unicode canonical caseless matching decomposes, folds and
                // decomposes again, because folding can expose new
                // decompositions. The chain owns its result, as the folded
                // form does not outlive this call.
                let decomposed = decompose(text);
                let folded = fold(&decomposed);
                Cow::Owned(
                    DecomposingNormalizerBorrowed::new_nfd()
                        .normalize_iter(folded.chars())
                        .collect(),
                )
            }
        }
    }
}

/// Returns `text` with Unicode default case folding applied.
#[cfg(feature = "alloc")]
fn fold(text: &str) -> Cow<'_, str> {
    CaseMapperBorrowed::new().fold_string(text)
}

/// Returns `text` in Unicode Normalization Form D (canonical decomposition).
#[cfg(feature = "alloc")]
fn decompose(text: &str) -> Cow<'_, str> {
    DecomposingNormalizerBorrowed::new_nfd().normalize(text)
}

/// Parses directory entries stored in `layout` from filesystem-tree leaf entries.
#[cfg(feature = "alloc")]
pub fn parse_directory_entries<'a>(
    entries: impl IntoIterator<Item = crate::types::FixedEntry<'a>>,
    parent_id: u64,
    layout: DirectoryKeyLayout,
) -> alloc::vec::Vec<DirectoryEntryRecord<'a>> {
    entries
        .into_iter()
        .filter_map(|entry| DirectoryEntryRecord::parse(entry.key, entry.value, layout).ok())
        .filter(|entry| entry.parent_id == parent_id)
        .collect()
}

/// Parses owned directory entries stored in `layout` from filesystem-tree
/// leaf entries.
#[cfg(feature = "alloc")]
pub fn parse_owned_directory_entries(
    entries: impl IntoIterator<Item = crate::types::OwnedEntry>,
    parent_id: u64,
    layout: DirectoryKeyLayout,
) -> alloc::vec::Vec<OwnedDirectoryEntryRecord> {
    entries
        .into_iter()
        .filter_map(|entry| OwnedDirectoryEntryRecord::parse(&entry.key, &entry.value, layout).ok())
        .filter(|entry| entry.parent_id == parent_id)
        .collect()
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn xattr_key(id: u64, name: &str) -> alloc::vec::Vec<u8> {
        let mut key = (id | (u64::from(FS_TYPE_XATTR) << 60))
            .to_le_bytes()
            .to_vec();
        key.extend_from_slice(&(name.len() as u16 + 1).to_le_bytes());
        key.extend_from_slice(name.as_bytes());
        key.push(0);
        key
    }

    fn drec(name: &[u8], hashed: bool) -> (alloc::vec::Vec<u8>, alloc::vec::Vec<u8>) {
        let mut key = (2 | (u64::from(FS_TYPE_DIRECTORY_RECORD) << 60))
            .to_le_bytes()
            .to_vec();
        if hashed {
            key.extend_from_slice(&((name.len() as u32 + 1) | (0x1234 << 10)).to_le_bytes());
        } else {
            key.extend_from_slice(&(name.len() as u16 + 1).to_le_bytes());
        }
        key.extend_from_slice(name);
        key.push(0);
        let mut value = 16_u64.to_le_bytes().to_vec();
        value.extend_from_slice(&0_u64.to_le_bytes());
        value.extend_from_slice(&DT_REG.to_le_bytes());
        (key, value)
    }

    #[test]
    fn directory_keys_parse_in_both_layouts() {
        for (hashed, layout) in [
            (true, DirectoryKeyLayout::Hashed),
            (false, DirectoryKeyLayout::Plain),
        ] {
            let (key, value) = drec(b"notes.txt", hashed);
            let entry = DirectoryEntryRecord::parse(&key, &value, layout).unwrap();
            assert_eq!(entry.parent_id, 2);
            assert_eq!(entry.file_id, 16);
            assert_eq!(entry.name, b"notes.txt");
            assert_eq!(entry.file_type(), DT_REG);
        }
        let (key, value) = drec(b"notes.txt", false);
        assert_ne!(
            DirectoryEntryRecord::parse(&key, &value, DirectoryKeyLayout::Hashed)
                .map(|entry| entry.name),
            Ok(&b"notes.txt"[..])
        );
    }

    #[test]
    fn directory_names_keep_bytes_that_are_not_utf8() {
        let name = b"caf\xe9\xff.txt";
        let (key, value) = drec(name, true);
        let entry = DirectoryEntryRecord::parse(&key, &value, DirectoryKeyLayout::Hashed).unwrap();
        assert_eq!(entry.name, name);
        assert_eq!(
            entry.name_str(),
            Err(crate::ApfsError::InvalidValue("directory name UTF-8"))
        );
        assert_eq!(
            OwnedDirectoryEntryRecord::parse(&key, &value, DirectoryKeyLayout::Hashed),
            Err(crate::ApfsError::InvalidValue("directory name UTF-8"))
        );
        let long = alloc::vec![0xe2; 1022];
        let (key, value) = drec(&long, true);
        let entry = DirectoryEntryRecord::parse(&key, &value, DirectoryKeyLayout::Hashed).unwrap();
        assert_eq!(entry.name.len(), 1022);
        let mut key = key;
        key.truncate(key.len() - 1);
        assert_eq!(
            DirectoryEntryRecord::parse(&key, &value, DirectoryKeyLayout::Hashed),
            Err(crate::ApfsError::InputTooSmall)
        );
    }

    #[test]
    fn embedded_xattr_data_is_parsed() {
        let key = xattr_key(21, SYMLINK_XATTR_NAME);
        let mut value = XATTR_DATA_EMBEDDED.to_le_bytes().to_vec();
        value.extend_from_slice(&10_u16.to_le_bytes());
        value.extend_from_slice(b"hello.txt\0");
        let xattr = XattrRecord::parse(&key, &value).unwrap();
        assert_eq!(xattr.id, 21);
        assert_eq!(xattr.name, SYMLINK_XATTR_NAME);
        assert_eq!(xattr.data, b"hello.txt\0");
    }

    #[test]
    fn xattr_data_past_the_record_is_rejected() {
        let key = xattr_key(21, "user.k");
        let mut value = XATTR_DATA_EMBEDDED.to_le_bytes().to_vec();
        value.extend_from_slice(&100_u16.to_le_bytes());
        value.extend_from_slice(b"short");
        assert_eq!(
            XattrRecord::parse(&key, &value).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
    }

    #[test]
    fn names_fold_case_only_on_case_insensitive_volumes() {
        let insensitive = NameComparison::new(true, false);
        let sensitive = NameComparison::new(false, false);
        assert!(insensitive.matches("Café.TXT", "café.txt"));
        assert!(!sensitive.matches("Café.TXT", "café.txt"));
        assert!(sensitive.matches("Café.TXT", "Café.TXT"));
        assert!(!insensitive.matches("a.txt", "b.txt"));
        // Full case folding, not lowercasing: both Greek sigmas and the
        // sharp s fold onto one name.
        assert!(insensitive.matches("\u{3a3}.txt", "\u{3c2}.txt"));
        assert!(insensitive.matches("stra\u{df}e.txt", "STRASSE.TXT"));
    }

    #[test]
    fn names_decompose_only_on_normalization_insensitive_volumes() {
        let insensitive = NameComparison::new(false, true);
        let sensitive = NameComparison::new(false, false);
        assert!(insensitive.matches("caf\u{e9}.txt", "cafe\u{301}.txt"));
        assert!(!sensitive.matches("caf\u{e9}.txt", "cafe\u{301}.txt"));
        assert!(!insensitive.matches("caf\u{e9}.txt", "caf\u{e8}.txt"));
        // Canonical caseless matching decomposes before folding as well.
        let both = NameComparison::new(true, true);
        assert!(both.matches("CAF\u{c9}.TXT", "cafe\u{301}.txt"));
        assert!(both.matches("CAFE\u{301}.txt", "caf\u{e9}.txt"));
    }

    #[test]
    fn stream_size_prefers_the_inode_size() {
        let mut inode = InodeRecord {
            id: 16,
            parent_id: 2,
            private_id: 16,
            mode: 0o100644,
            bsd_flags: 0,
            owner: 0,
            group: 0,
            uncompressed_size: 0,
            link_or_child_count: 1,
            create_time_ns: 0,
            modification_time_ns: 0,
            change_time_ns: 0,
            access_time_ns: 0,
            data_stream_size: None,
        };
        let extents = [FileExtentRecord {
            id: 16,
            logical_address: 8192,
            length: 4096,
            flags: 0,
            physical_block: 0,
            cryptography_id: 0,
        }];
        assert_eq!(stream_size(&inode, &extents), 12288);
        inode.uncompressed_size = 100;
        assert_eq!(stream_size(&inode, &extents), 100);
        inode.data_stream_size = Some(5000);
        assert_eq!(stream_size(&inode, &extents), 5000);
    }

    fn inode_key(id: u64) -> alloc::vec::Vec<u8> {
        (id | (u64::from(FS_TYPE_INODE) << 60))
            .to_le_bytes()
            .to_vec()
    }

    /// An inode value whose one extended field is a `j_dstream_t` declaring
    /// `size_bytes` bytes and holding `data`.
    fn inode_value(size_bytes: u16, data: &[u8]) -> alloc::vec::Vec<u8> {
        let mut value = alloc::vec![0_u8; INODE_FIXED_VALUE_SIZE];
        value.extend_from_slice(&1_u16.to_le_bytes());
        value.extend_from_slice(&0_u16.to_le_bytes());
        value.push(INODE_EXT_TYPE_DATA_STREAM);
        value.push(0);
        value.extend_from_slice(&size_bytes.to_le_bytes());
        value.extend_from_slice(data);
        value
    }

    #[test]
    fn malformed_inode_extended_fields_are_rejected() {
        let key = inode_key(16);
        let mut data = alloc::vec![0_u8; 40];
        data[..8].copy_from_slice(&4242_u64.to_le_bytes());
        assert_eq!(
            InodeRecord::parse(&key, &inode_value(40, &data))
                .unwrap()
                .data_stream_size,
            Some(4242)
        );

        // A field whose declared data runs past the record is corrupt, not
        // an inode without a data stream.
        assert_eq!(
            InodeRecord::parse(&key, &inode_value(40, &data[..2])).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
        // A data-stream field too short for `j_dstream_t` holds no size.
        assert_eq!(
            InodeRecord::parse(&key, &inode_value(4, &data[..8])).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
        // A blob without room for its field table is corrupt too.
        let mut header_only = alloc::vec![0_u8; INODE_FIXED_VALUE_SIZE];
        header_only.extend_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            InodeRecord::parse(&key, &header_only).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
        // An inode with no extended fields records no size.
        let bare = alloc::vec![0_u8; INODE_FIXED_VALUE_SIZE];
        assert_eq!(
            InodeRecord::parse(&key, &bare).unwrap().data_stream_size,
            None
        );
    }
}

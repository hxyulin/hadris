//! APFS filesystem-tree record parsing.

use crate::types::le_u64;

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

/// Parsed directory entry record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryRecord<'a> {
    /// Parent directory inode identifier.
    pub parent_id: u64,
    /// Child inode identifier.
    pub file_id: u64,
    /// Directory entry flags; the bits in [`DREC_TYPE_MASK`] are the file type.
    pub flags: u16,
    /// Entry name.
    pub name: &'a str,
}

impl<'a> DirectoryEntryRecord<'a> {
    /// Parses a directory entry key/value pair.
    pub fn parse(key: &'a [u8], value: &'a [u8]) -> crate::Result<Self> {
        let header = FileSystemKey::parse(key)?;
        if header.record_type != FS_TYPE_DIRECTORY_RECORD {
            return Err(crate::ApfsError::InvalidValue("directory record key type"));
        }
        let name_len_and_hash = u32::from_le_bytes(crate::types::take(key, 8)?);
        let name_len = (name_len_and_hash & 0x3ff) as usize;
        let name_bytes = key
            .get(12..12 + name_len)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        let name_bytes = name_bytes.strip_suffix(&[0]).unwrap_or(name_bytes);
        Ok(Self {
            parent_id: header.id,
            file_id: le_u64(value, 0)?,
            flags: u16::from_le_bytes(crate::types::take(value, 16)?),
            name: core::str::from_utf8(name_bytes)
                .map_err(|_| crate::ApfsError::InvalidValue("directory name UTF-8"))?,
        })
    }

    /// Returns the file type, such as [`DT_DIR`], [`DT_REG`] or [`DT_LNK`].
    pub const fn file_type(&self) -> u16 {
        self.flags & DREC_TYPE_MASK
    }
}

/// Owned parsed directory entry record.
#[cfg(any(feature = "alloc", feature = "std"))]
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[cfg(any(feature = "alloc", feature = "std"))]
impl OwnedDirectoryEntryRecord {
    /// Parses a directory entry key/value pair into an owned record.
    pub fn parse(key: &[u8], value: &[u8]) -> crate::Result<Self> {
        let entry = DirectoryEntryRecord::parse(key, value)?;
        Ok(Self {
            parent_id: entry.parent_id,
            file_id: entry.file_id,
            flags: entry.flags,
            name: entry.name.into(),
        })
    }

    /// Returns the file type, such as [`DT_DIR`], [`DT_REG`] or [`DT_LNK`].
    pub const fn file_type(&self) -> u16 {
        self.flags & DREC_TYPE_MASK
    }
}

/// Small parsed inode summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            data_stream_size: parse_inode_data_stream_size(value),
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

/// Parses the `size` field of an inode's `j_dstream_t` extended field, if present.
fn parse_inode_data_stream_size(value: &[u8]) -> Option<u64> {
    let blob = value.get(INODE_FIXED_VALUE_SIZE..)?;
    let count = u16::from_le_bytes(blob.get(0..2)?.try_into().ok()?) as usize;
    let fields = blob.get(4..)?;
    let mut field_offset = 0usize;
    let mut data_offset = count.checked_mul(4)?;
    for _ in 0..count {
        let entry = fields.get(field_offset..field_offset + 4)?;
        let field_type = entry[0];
        let size_bytes = u16::from_le_bytes(entry[2..4].try_into().ok()?) as usize;
        field_offset += 4;
        let data = fields.get(data_offset..data_offset.checked_add(size_bytes)?)?;
        if field_type == INODE_EXT_TYPE_DATA_STREAM {
            return le_u64(data, 0).ok();
        }
        data_offset += size_bytes;
        let remainder = size_bytes % 8;
        if remainder != 0 {
            data_offset += 8 - remainder;
        }
    }
    None
}

/// Parsed file extent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Compares a stored directory entry name with a looked-up name the way the
/// volume does: with Unicode case folding when `case_insensitive` is set.
/// Differences in Unicode normalization are not folded.
pub fn names_match(stored: &str, wanted: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        stored
            .chars()
            .flat_map(char::to_lowercase)
            .eq(wanted.chars().flat_map(char::to_lowercase))
    } else {
        stored == wanted
    }
}

/// Parses directory entries from filesystem-tree leaf entries.
#[cfg(any(feature = "alloc", feature = "std"))]
pub fn parse_directory_entries<'a>(
    entries: impl IntoIterator<Item = crate::types::FixedEntry<'a>>,
    parent_id: u64,
) -> alloc::vec::Vec<DirectoryEntryRecord<'a>> {
    entries
        .into_iter()
        .filter_map(|entry| DirectoryEntryRecord::parse(entry.key, entry.value).ok())
        .filter(|entry| entry.parent_id == parent_id)
        .collect()
}

/// Parses owned directory entries from filesystem-tree leaf entries.
#[cfg(any(feature = "alloc", feature = "std"))]
pub fn parse_owned_directory_entries(
    entries: impl IntoIterator<Item = crate::types::OwnedEntry>,
    parent_id: u64,
) -> alloc::vec::Vec<OwnedDirectoryEntryRecord> {
    entries
        .into_iter()
        .filter_map(|entry| OwnedDirectoryEntryRecord::parse(&entry.key, &entry.value).ok())
        .filter(|entry| entry.parent_id == parent_id)
        .collect()
}

#[cfg(all(test, any(feature = "alloc", feature = "std")))]
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
        assert!(names_match("Café.TXT", "café.txt", true));
        assert!(!names_match("Café.TXT", "café.txt", false));
        assert!(names_match("Café.TXT", "Café.TXT", false));
        assert!(!names_match("a.txt", "b.txt", true));
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
}

//! APFS filesystem-tree record parsing.

use crate::types::le_u64;

/// Root directory inode number.
pub const INODE_ROOT_DIRECTORY: u64 = 2;
/// Filesystem record type for inode records.
pub const FS_TYPE_INODE: u8 = 3;
/// Filesystem record type for file extent records.
pub const FS_TYPE_FILE_EXTENT: u8 = 8;
/// Filesystem record type for directory entries.
pub const FS_TYPE_DIRECTORY_RECORD: u8 = 9;

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
    /// Directory entry flags; low byte is the file type.
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
}

/// Owned parsed directory entry record.
#[cfg(any(feature = "alloc", feature = "std"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedDirectoryEntryRecord {
    /// Parent directory inode identifier.
    pub parent_id: u64,
    /// Child inode identifier.
    pub file_id: u64,
    /// Directory entry flags; low byte is the file type.
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
            mode: u16::from_le_bytes(crate::types::take(value, 80)?),
            uncompressed_size: le_u64(value, 84)?,
            data_stream_size: parse_inode_data_stream_size(value),
        })
    }
}

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

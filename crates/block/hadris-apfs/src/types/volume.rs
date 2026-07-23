//! APFS volume superblock parsing.

use crate::types::object::{ObjectHeader, ObjectType};
use crate::types::{le_u32, le_u64, take};

/// Magic value in volume superblocks (`APSB` on disk).
pub const VOLUME_MAGIC: [u8; 4] = *b"APSB";
/// Length of the APFS volume name field.
pub const VOLUME_NAME_LENGTH: usize = 256;

/// Parsed APFS volume superblock (`apfs_superblock_t`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSuperblock {
    /// Common object header.
    pub object: ObjectHeader,
    /// Volume index in the container volume array.
    pub fs_index: u32,
    /// Optional feature flags.
    pub optional_features: u64,
    /// Read-only compatible feature flags.
    pub readonly_compatible_features: u64,
    /// Incompatible feature flags.
    pub incompatible_features: u64,
    /// Blocks allocated by the volume.
    pub allocated_block_count: u64,
    /// Volume object map physical OID.
    pub object_map_oid: u64,
    /// Root filesystem tree OID.
    pub root_tree_oid: u64,
    /// Extent-reference tree OID.
    pub extent_reference_tree_oid: u64,
    /// Snapshot metadata tree OID.
    pub snapshot_metadata_tree_oid: u64,
    /// Number of regular files.
    pub number_files: u64,
    /// Number of directories.
    pub number_directories: u64,
    /// Number of symbolic links.
    pub number_symlinks: u64,
    /// Number of snapshots.
    pub number_snapshots: u64,
    /// Volume UUID.
    pub volume_id: [u8; 16],
    /// Volume flags.
    pub flags: u64,
    /// Null-terminated UTF-8 volume name bytes.
    pub volume_name: [u8; VOLUME_NAME_LENGTH],
}

impl VolumeSuperblock {
    /// Parses a volume superblock from bytes beginning at an APFS object.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let object = ObjectHeader::parse(data)?;
        if object.kind() != ObjectType::VolumeSuperblock as u16 {
            return Err(crate::ApfsError::InvalidValue(
                "volume superblock object type",
            ));
        }
        let magic = take::<4>(data, 32)?;
        if magic != VOLUME_MAGIC {
            return Err(crate::ApfsError::InvalidMagic {
                expected: VOLUME_MAGIC,
                actual: magic,
            });
        }
        Ok(Self {
            object,
            fs_index: le_u32(data, 36)?,
            optional_features: le_u64(data, 40)?,
            readonly_compatible_features: le_u64(data, 48)?,
            incompatible_features: le_u64(data, 56)?,
            allocated_block_count: le_u64(data, 88)?,
            object_map_oid: le_u64(data, 128)?,
            root_tree_oid: le_u64(data, 136)?,
            extent_reference_tree_oid: le_u64(data, 144)?,
            snapshot_metadata_tree_oid: le_u64(data, 152)?,
            number_files: le_u64(data, 184)?,
            number_directories: le_u64(data, 192)?,
            number_symlinks: le_u64(data, 200)?,
            number_snapshots: le_u64(data, 216)?,
            volume_id: take(data, 240)?,
            flags: le_u64(data, 264)?,
            volume_name: take(data, 704)?,
        })
    }

    /// Returns the volume name as UTF-8 up to the first NUL byte.
    pub fn name(&self) -> crate::Result<&str> {
        let len = self
            .volume_name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(VOLUME_NAME_LENGTH);
        core::str::from_utf8(&self.volume_name[..len])
            .map_err(|_| crate::ApfsError::InvalidValue("volume name UTF-8"))
    }
}

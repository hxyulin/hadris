//! APFS object headers.

use crate::types::{le_u32, le_u64};

/// Size of every APFS object header in bytes.
pub const OBJECT_HEADER_SIZE: usize = 32;

/// Common APFS object header (`obj_phys_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHeader {
    /// Stored Fletcher-64 checksum.
    pub checksum: u64,
    /// Object identifier.
    pub identifier: u64,
    /// Transaction identifier.
    pub transaction_identifier: u64,
    /// Object type value.
    pub object_type: u32,
    /// Object subtype value.
    pub subtype: u32,
}

impl ObjectHeader {
    /// Parses an object header from the beginning of `data`.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        Ok(Self {
            checksum: le_u64(data, 0)?,
            identifier: le_u64(data, 8)?,
            transaction_identifier: le_u64(data, 16)?,
            object_type: le_u32(data, 24)?,
            subtype: le_u32(data, 28)?,
        })
    }

    /// Low 16-bit APFS object type discriminator.
    pub const fn kind(self) -> u16 {
        (self.object_type & 0xffff) as u16
    }
}

/// Known APFS object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ObjectType {
    /// Invalid or absent object subtype.
    Invalid = 0,
    /// Container superblock (`nx_superblock_t`).
    ContainerSuperblock = 1,
    /// B-tree root node.
    BTreeRoot = 2,
    /// B-tree node.
    BTreeNode = 3,
    /// Space manager.
    SpaceManager = 5,
    /// Space manager chunk-info block (`chunk_info_block_t`).
    SpaceManagerChunkInformationBlock = 7,
    /// Object map.
    ObjectMap = 11,
    /// Checkpoint map.
    CheckpointMap = 12,
    /// Volume superblock (`apfs_superblock_t`).
    VolumeSuperblock = 13,
}

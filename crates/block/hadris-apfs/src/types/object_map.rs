//! APFS object map structures.

use crate::types::object::{ObjectHeader, ObjectType};
use crate::types::{le_u32, le_u64};

/// Parsed object map block (`omap_phys_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMapBlock {
    /// Common object header.
    pub object: ObjectHeader,
    /// Object map flags.
    pub flags: u32,
    /// Number of snapshots recorded in the object map.
    pub snapshot_count: u32,
    /// Raw APFS object type used by the object-mapping tree.
    pub tree_type: u32,
    /// Raw APFS object type used by the snapshot tree.
    pub snapshot_tree_type: u32,
    /// Object identifier of the object-mapping B-tree root.
    pub tree_oid: u64,
    /// Object identifier of the snapshot B-tree root.
    pub snapshot_tree_oid: u64,
    /// Most recent snapshot transaction identifier.
    pub most_recent_snapshot_identifier: u64,
    /// Minimum transaction identifier for a pending revert.
    pub pending_revert_minimum_identifier: u64,
    /// Maximum transaction identifier for a pending revert.
    pub pending_revert_maximum_identifier: u64,
}

impl ObjectMapBlock {
    /// Parses an object map block from bytes.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let object = ObjectHeader::parse(data)?;
        if object.kind() != ObjectType::ObjectMap as u16 {
            return Err(crate::ApfsError::InvalidValue("object map object type"));
        }
        Ok(Self {
            object,
            flags: le_u32(data, 32)?,
            snapshot_count: le_u32(data, 36)?,
            tree_type: le_u32(data, 40)?,
            snapshot_tree_type: le_u32(data, 44)?,
            tree_oid: le_u64(data, 48)?,
            snapshot_tree_oid: le_u64(data, 56)?,
            most_recent_snapshot_identifier: le_u64(data, 64)?,
            pending_revert_minimum_identifier: le_u64(data, 72)?,
            pending_revert_maximum_identifier: le_u64(data, 80)?,
        })
    }
}

/// Object map lookup key (`omap_key_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectMapKey {
    /// Virtual object identifier.
    pub oid: u64,
    /// Transaction identifier.
    pub xid: u64,
}

/// Object map lookup value (`omap_val_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMapValue {
    /// Value flags.
    pub flags: u32,
    /// Object size in bytes.
    pub size: u32,
    /// Physical APFS block address.
    pub address: u64,
}

impl ObjectMapValue {
    /// Parses an object map value from bytes.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        Ok(Self {
            flags: le_u32(data, 0)?,
            size: le_u32(data, 4)?,
            address: le_u64(data, 8)?,
        })
    }
}

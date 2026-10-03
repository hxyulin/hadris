//! Common APFS scalar types.

/// APFS object identifier.
pub type ObjectId = u64;
/// APFS transaction identifier.
pub type TransactionId = u64;
/// APFS physical block address.
pub type PhysicalAddress = u64;
/// APFS UUID bytes as stored on disk.
pub type Uuid = [u8; 16];

/// Physical block range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalAddressRange {
    /// Starting physical block.
    pub start: PhysicalAddress,
    /// Number of blocks in the range.
    pub count: u64,
}

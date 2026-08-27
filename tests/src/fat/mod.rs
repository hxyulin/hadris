//! FAT12/16/32 conformance model, oracle, scenarios, and implementation adapters.

pub mod adapter;
pub mod fatfs;
pub mod hadris;
pub mod limits;
pub mod model;
pub mod mtools;
pub mod native;
pub mod scenarios;
pub mod spec;

pub use adapter::{
    FatAdapter, apply_operations, apply_operations_without_attrs, apply_rejection,
    clear_mutable_attrs,
};
pub use model::{
    EntryState, FsState, Operation, compare_snapshot, format_trace, summarize_operation,
};

/// Report directory name under the harness report root.
pub const FORMAT: &str = "fat";
pub const LABEL: &str = "HADRISCONF";
pub const READ_ONLY: u8 = 0x01;
pub const HIDDEN: u8 = 0x02;
pub const SYSTEM: u8 = 0x04;
pub const ARCHIVE: u8 = 0x20;
/// Attribute bits that every implementation is expected to preserve.
pub const MUTABLE_ATTRS: u8 = READ_ONLY | HIDDEN | SYSTEM | ARCHIVE;

/// One FAT width with an image size that forces that width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FatCase {
    pub name: &'static str,
    pub size: u64,
    pub bits: u8,
}

impl FatCase {
    /// The width as `mkfs.fat -F` / `newfs_msdos -F` expect it.
    pub fn mkfs_type(&self) -> String {
        self.bits.to_string()
    }
}

pub const FAT_CASES: [FatCase; 3] = [
    FatCase {
        name: "fat12",
        size: 2 * 1024 * 1024,
        bits: 12,
    },
    FatCase {
        name: "fat16",
        size: 16 * 1024 * 1024,
        bits: 16,
    },
    FatCase {
        name: "fat32",
        size: 64 * 1024 * 1024,
        bits: 32,
    },
];

pub fn fat_path_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

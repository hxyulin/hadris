//! exFAT conformance oracle, scenarios, limits and implementation adapters.
//!
//! exFAT shares the FAT operation model and adapter trait: the operations,
//! the archive rule and case-insensitive names are the same, and the
//! attribute bits the model tracks have the same values in `FileAttributes`.
//! What differs is the on-disk oracle, the formatter and the peers.

pub mod generic;
pub mod limits;
pub mod native;
pub mod scenarios;
pub mod spec;

/// Report directory name under the harness report root.
pub const FORMAT: &str = "exfat";

/// One exFAT layout: a volume size, a cluster size and the number of FATs,
/// 2 for TexFAT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExFatCase {
    pub name: &'static str,
    pub size: u64,
    pub cluster: u32,
    pub fats: u8,
}

pub const EXFAT_CASES: [ExFatCase; 4] = [
    ExFatCase {
        name: "exfat-512",
        size: 4 * 1024 * 1024,
        cluster: 512,
        fats: 1,
    },
    ExFatCase {
        name: "exfat-4k",
        size: 16 * 1024 * 1024,
        cluster: 4096,
        fats: 1,
    },
    ExFatCase {
        name: "exfat-32k",
        size: 64 * 1024 * 1024,
        cluster: 32 * 1024,
        fats: 1,
    },
    ExFatCase {
        name: "texfat-4k",
        size: 16 * 1024 * 1024,
        cluster: 4096,
        fats: 2,
    },
];

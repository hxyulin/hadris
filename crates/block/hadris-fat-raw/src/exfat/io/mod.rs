//! exFAT device primitives, generated for each mode from one source.
//!
//! The functions in `sync`, `r#async` (`Send` futures) and `local`
//! (futures need not be `Send`) borrow the caller's [`BlockBuf`](crate::io::BlockBuf)
//! and an [`ExFat`], which holds what a driver tracks about the volume
//! between calls: its flags, its Allocation Bitmaps and the free count. The
//! up-case table is an [`Upcase`] index the caller owns.
//!
//! Sequences whose order matters when they are interrupted live here once.
//! The first write after mounting sets `VolumeDirty`. FAT entries and
//! bitmap bits are written to the inactive FAT and bitmap of a TexFAT
//! volume first. `allocate_run` links the FAT before it sets any bit, and
//! `free_chain` clears bits a device block at a time, recording its
//! progress in a [`Held`](crate::io::Held). `write_set` writes the entries
//! of a set that share a device block with one write, the File entry's
//! block last; `clear_set` writes the File entry's block first.

use crate::exfat::{FIRST_CLUSTER, Geometry, PageStart};
use crate::io::ChainPos;

/// Decoded up-case pages kept besides page 0.
const UPCASE_CACHE: usize = 2;

/// A chain of clusters holding a system structure: an Allocation Bitmap or
/// the up-case table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    first: u32,
    len: u64,
    contiguous: bool,
}

impl Extent {
    /// `len` bytes in the FAT chain at `first`.
    pub const fn chain(first: u32, len: u64) -> Self {
        Self {
            first,
            len,
            contiguous: false,
        }
    }

    /// `len` bytes in consecutive clusters from `first`.
    pub const fn contiguous(first: u32, len: u64) -> Self {
        Self {
            first,
            len,
            contiguous: true,
        }
    }

    /// The first cluster.
    pub const fn first(&self) -> u32 {
        self.first
    }

    /// The length in bytes.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the length is 0.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the clusters follow one another on disk.
    pub const fn is_contiguous(&self) -> bool {
        self.contiguous
    }
}

/// The up-case table, indexed once at mount and decoded lazily: where each
/// 256-unit page starts, which pages map every unit to itself, page 0, and
/// a few more decoded pages.
#[derive(Debug, Clone)]
pub struct Upcase {
    extent: Extent,
    checksum: u32,
    stored_checksum: u32,
    starts: [PageStart; 256],
    identity: [u8; 32],
    page0: [u16; 256],
    cache: [(u16, [u16; 256]); UPCASE_CACHE],
    victim: usize,
}

impl Upcase {
    /// An empty index, for `read_volume` to fill.
    pub fn new() -> Self {
        Self {
            extent: Extent::contiguous(0, 0),
            checksum: 0,
            stored_checksum: 0,
            starts: [PageStart::default(); 256],
            identity: [0xFF; 32],
            page0: [0; 256],
            cache: [(0, [0; 256]); UPCASE_CACHE],
            victim: 0,
        }
    }

    /// Where the table is.
    pub const fn extent(&self) -> Extent {
        self.extent
    }

    /// The checksum of the table's bytes.
    pub const fn checksum(&self) -> u32 {
        self.checksum
    }

    /// The `TableChecksum` of the Up-case Table entry.
    pub const fn stored_checksum(&self) -> u32 {
        self.stored_checksum
    }

    /// Whether the table matches its checksum and maps the first 128 code
    /// points as the specification requires.
    pub fn is_valid(&self) -> bool {
        self.checksum == self.stored_checksum
            && (0..128u16)
                .all(|code| self.page0[code as usize] == crate::exfat::mandatory_upcase(code))
    }

    pub(crate) fn is_identity(&self, page: usize) -> bool {
        self.identity[page / 8] & (1 << (page % 8)) != 0
    }
}

impl Default for Upcase {
    fn default() -> Self {
        Self::new()
    }
}

/// A position in a directory's clusters, kept across slots so a scan walks
/// the chain once.
#[derive(Debug, Clone, Copy)]
pub struct DirWalk {
    dir: Extent,
    at: ChainPos,
}

impl DirWalk {
    /// A walk of the directory whose entries `dir` holds. Its length is the
    /// directory's `DataLength`, or `u64::MAX` for the root directory,
    /// which ends with its chain.
    pub const fn new(dir: Extent) -> Self {
        Self {
            dir,
            at: ChainPos::NONE,
        }
    }

    /// A walk that resumes from `at`, a position in the directory's chain.
    pub const fn resume(dir: Extent, at: ChainPos) -> Self {
        Self { dir, at }
    }

    /// The directory walked.
    pub const fn dir(&self) -> Extent {
        self.dir
    }

    /// The position its chain was last walked to.
    pub const fn pos(&self) -> ChainPos {
        self.at
    }
}

/// Which boot region a volume was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootRegion {
    /// The main boot region.
    Main,
    /// The backup boot region, because the main one is damaged.
    Backup,
}

/// The state of a bitmap bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterState {
    /// The cluster is free.
    Free,
    /// The cluster is allocated.
    Used,
}

/// Whether `VolumeDirty` was set, and by whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dirty {
    /// Clear on disk.
    Clean,
    /// Set by a write since mounting or the last `clear_dirty`.
    Marked,
    /// Set at mount, and left for a checker to clear.
    Inherited,
}

/// What a driver tracks about an exFAT volume between calls: its geometry,
/// `VolumeFlags` as last written, the Allocation Bitmap of the active FAT
/// and, on a TexFAT volume, of the other one, and the free count and
/// allocation hint.
#[derive(Debug, Clone, Copy)]
pub struct ExFat {
    geo: Geometry,
    flags: u16,
    dirty: Dirty,
    bitmap: Extent,
    mirror: Option<Extent>,
    hint: ChainPos,
    free: Option<u32>,
    next_free: u32,
    changed: bool,
}

impl ExFat {
    /// The state of a volume with geometry `geo` whose active Allocation
    /// Bitmap is `bitmap`, with `mirror` the bitmap of the other FAT of a
    /// TexFAT volume.
    pub const fn new(geo: Geometry, bitmap: Extent, mirror: Option<Extent>) -> Self {
        Self {
            geo,
            flags: geo.flags(),
            dirty: if geo.flags() & crate::exfat::VOLUME_DIRTY != 0 {
                Dirty::Inherited
            } else {
                Dirty::Clean
            },
            bitmap,
            mirror,
            hint: ChainPos::NONE,
            free: None,
            next_free: FIRST_CLUSTER,
            changed: false,
        }
    }

    /// The volume's geometry.
    pub const fn geometry(&self) -> &Geometry {
        &self.geo
    }

    /// Records a serial written to both boot regions.
    pub fn set_volume_serial(&mut self, serial: u32) {
        self.geo.set_volume_serial(serial);
    }

    /// `VolumeFlags` as last written.
    pub const fn flags(&self) -> u16 {
        self.flags
    }

    /// Whether `VolumeDirty` was set at mount.
    pub const fn was_dirty(&self) -> bool {
        matches!(self.dirty, Dirty::Inherited)
    }

    /// The Allocation Bitmap of the active FAT.
    pub const fn bitmap(&self) -> Extent {
        self.bitmap
    }

    /// The Allocation Bitmap of the other FAT of a TexFAT volume.
    pub const fn mirror_bitmap(&self) -> Option<Extent> {
        self.mirror
    }

    /// The number of free clusters, when known.
    pub const fn free_clusters(&self) -> Option<u32> {
        self.free
    }

    /// Whether a bitmap bit changed since `PercentInUse` was last written.
    pub const fn allocation_changed(&self) -> bool {
        self.changed
    }

    fn adjust_free(&mut self, flips: u32, state: ClusterState) {
        self.changed = true;
        self.free = match (self.free, state) {
            (Some(free), ClusterState::Used) => free.checked_sub(flips),
            (Some(free), ClusterState::Free) => Some((free + flips).min(self.geo.cluster_count())),
            (None, _) => None,
        };
    }
}

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous exFAT primitives.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use crate::io::sync as block;
    use hadris_storage::sync as storage;

    #[path = "check.rs"]
    mod check;
    #[path = "volume.rs"]
    mod volume;

    pub use check::check;

    pub use volume::{
        allocate, allocate_run, begin_write, bit, bitmap_bytes, clear_dirty, clear_set, count_free,
        free_chain, get, next, read_backup_boot, read_boot, read_volume, set, set_bit, slot_offset,
        upcase, write_percent_in_use, write_set,
    };
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous exFAT primitives with `Send` futures, for devices
    //! whose futures are `Send`.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    use crate::io::r#async as block;
    use hadris_storage::r#async as storage;

    #[path = "check.rs"]
    mod check;
    #[path = "volume.rs"]
    mod volume;

    pub use check::check;

    pub use volume::{
        allocate, allocate_run, begin_write, bit, bitmap_bytes, clear_dirty, clear_set, count_free,
        free_chain, get, next, read_backup_boot, read_boot, read_volume, set, set_bit, slot_offset,
        upcase, write_percent_in_use, write_set,
    };
}

#[cfg(feature = "async")]
#[path = ""]
pub mod local {
    //! The asynchronous exFAT primitives.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use crate::io::local as block;
    use hadris_storage::local as storage;

    #[path = "check.rs"]
    mod check;
    #[path = "volume.rs"]
    mod volume;

    pub use check::check;

    pub use volume::{
        allocate, allocate_run, begin_write, bit, bitmap_bytes, clear_dirty, clear_set, count_free,
        free_chain, get, next, read_backup_boot, read_boot, read_volume, set, set_bit, slot_offset,
        upcase, write_percent_in_use, write_set,
    };
}

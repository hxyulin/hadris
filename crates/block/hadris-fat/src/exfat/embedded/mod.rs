//! The embedded exFAT reader: [`sync::ExFat`] and `r#async::ExFat`, a
//! handle-based, read-only exFAT driver for firmware without an allocator,
//! such as SDXC card readers.
//!
//! `ExFat<D, const FILES: usize = 4>` is a separate type from the FAT
//! `Fat`, so FAT-only firmware does not link it. It is built on the
//! `hadris-fat-raw` exFAT primitives, keeps one 512-byte block buffer, the
//! geometry, the [`Options`] and `FILES` file slots, and takes 512-byte
//! device blocks only. It never writes: exFAT writes for firmware come in
//! 3.x through a new entry point, and `mount` stays read-only.
//!
//! Names compare with the fold of the [`Options`], ASCII by default, not
//! through the volume's up-case table, which would not fit the budget.
//! `Options::new().with_fold(hadris_fat_raw::fold_unicode)` matches every
//! name a Windows up-case table does in the Basic Multilingual Plane.

use hadris_fat_raw::exfat::{self as raw, RawEntry};
use hadris_fat_raw::io::ChainPos;
use hadris_fs::{Attributes, DirCursor, FileType, Metadata};

pub use crate::embedded::{File, Options};

/// A directory: the root or a subdirectory by its allocation. `Copy` and
/// holds no slot; it stays valid while the directory exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dir {
    /// First cluster, 0 for the root.
    first: u32,
    /// `DataLength`.
    len: u64,
    contiguous: bool,
}

impl Dir {
    pub(crate) const ROOT: Self = Self {
        first: 0,
        len: 0,
        contiguous: false,
    };

    pub(crate) const fn new(first: u32, len: u64, contiguous: bool) -> Self {
        Self {
            first,
            len,
            contiguous,
        }
    }

    /// Whether this is the root directory.
    pub const fn is_root(self) -> bool {
        self.first == 0
    }

    /// The directory's entries as a raw extent, the root's from `root`.
    pub(crate) fn extent(self, root: u32) -> hadris_fat_raw::exfat::io::Extent {
        use hadris_fat_raw::exfat::io::Extent;
        if self.is_root() {
            Extent::chain(root, u64::MAX)
        } else if self.contiguous {
            Extent::contiguous(self.first, self.len)
        } else {
            Extent::chain(self.first, self.len)
        }
    }
}

/// Where a listed entry set is: its directory and slot, valid for
/// `open_node` until the directory changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Node {
    dir: Dir,
    slot: u32,
}

impl Node {
    pub(crate) const fn new(dir: Dir, slot: u32) -> Self {
        Self { dir, slot }
    }

    pub(crate) const fn dir(self) -> Dir {
        self.dir
    }

    pub(crate) const fn slot(self) -> u32 {
        self.slot
    }
}

pub(crate) fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

pub(crate) fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

/// The File and Stream Extension entries of a set, which hold everything
/// but its name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Head {
    pub(crate) primary: RawEntry,
    pub(crate) stream: RawEntry,
}

impl Head {
    pub(crate) fn attributes(&self) -> u16 {
        le16(&self.primary, 4)
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.attributes() & raw::ATTR_DIRECTORY != 0
    }

    pub(crate) fn first(&self) -> u32 {
        le32(&self.stream, 20)
    }

    pub(crate) fn len(&self) -> u64 {
        le64(&self.stream, 24)
    }

    pub(crate) fn valid(&self) -> u64 {
        le64(&self.stream, 8).min(self.len())
    }

    pub(crate) fn contiguous(&self) -> bool {
        self.stream[1] & raw::NO_FAT_CHAIN != 0
    }

    pub(crate) fn as_dir(&self) -> Dir {
        Dir::new(self.first(), self.len(), self.contiguous())
    }

    fn metadata(&self, zone: Option<i16>) -> Metadata {
        let primary = &self.primary;
        let mut attributes = Attributes::empty();
        for (bit, flag) in [
            (raw::ATTR_READ_ONLY, Attributes::READ_ONLY),
            (raw::ATTR_HIDDEN, Attributes::HIDDEN),
            (raw::ATTR_SYSTEM, Attributes::SYSTEM),
            (raw::ATTR_ARCHIVE, Attributes::ARCHIVE),
        ] {
            if self.attributes() & bit != 0 {
                attributes |= flag;
            }
        }
        let dir = self.is_dir();
        let (file_type, len) = if dir {
            (FileType::Dir, 0)
        } else {
            (FileType::File, self.len())
        };
        let read_only = attributes.contains(Attributes::READ_ONLY);
        let mut meta = Metadata::new(file_type, crate::names::permissions(dir, read_only))
            .with_len(len)
            .with_attributes(attributes);
        if let Some(time) = raw::decode_time(le32(primary, 8), primary[20], primary[22], zone) {
            meta = meta.with_created(time);
        }
        if let Some(time) = raw::decode_time(le32(primary, 12), primary[21], primary[23], zone) {
            meta = meta.with_modified(time);
        }
        if let Some(time) = raw::decode_time(le32(primary, 16), 0, primary[24], zone) {
            meta = meta.with_accessed(time);
        }
        meta
    }
}

/// One directory entry set, lent to the callback of `list`.
#[derive(Debug)]
pub struct Entry<'a> {
    name: &'a [u16],
    head: Head,
    node: Node,
    zone: Option<i16>,
}

impl<'a> Entry<'a> {
    pub(crate) const fn new(name: &'a [u16], head: Head, node: Node, zone: Option<i16>) -> Self {
        Self {
            name,
            head,
            node,
            zone,
        }
    }

    /// The name in UTF-16.
    pub fn name_utf16(&self) -> &'a [u16] {
        self.name
    }

    /// The characters of the name, with unpaired surrogates as U+FFFD.
    pub fn chars(&self) -> impl Iterator<Item = char> + 'a {
        hadris_fat_raw::name::utf16_chars(self.name.iter().copied())
    }

    /// File or directory.
    pub fn file_type(&self) -> FileType {
        if self.head.is_dir() {
            FileType::Dir
        } else {
            FileType::File
        }
    }

    /// Whether the entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.head.is_dir()
    }

    /// The file's `DataLength`; 0 for a directory.
    pub fn len(&self) -> u64 {
        if self.head.is_dir() {
            0
        } else {
            self.head.len()
        }
    }

    /// Whether [`len`](Self::len) is 0.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The file attributes.
    pub fn attributes(&self) -> Attributes {
        self.metadata().attributes()
    }

    /// The metadata the entry set stores.
    pub fn metadata(&self) -> Metadata {
        self.head.metadata(self.zone)
    }

    /// The entry set's position, for `open_node`.
    pub fn node(&self) -> Node {
        self.node
    }

    /// Where a `list` continues after this entry set.
    pub fn next_cursor(&self) -> DirCursor {
        DirCursor::from_raw(self.node.slot as u64 + 1 + self.head.primary[1] as u64)
    }
}

pub(crate) fn metadata(head: &Head, zone: Option<i16>) -> Metadata {
    head.metadata(zone)
}

/// One open file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileSlot {
    pub(crate) head: Option<Head>,
    pub(crate) pos: u64,
    pub(crate) at: ChainPos,
    pub(crate) generation: u16,
}

impl FileSlot {
    pub(crate) const FREE: Self = Self {
        head: None,
        pos: 0,
        at: ChainPos::NONE,
        generation: 0,
    };
}

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous embedded exFAT reader, over a
    //! `hadris_storage::sync::BlockDevice`.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_fat_raw::exfat::io::sync as exio;
    use hadris_fat_raw::io::sync as rawio;
    use hadris_storage::sync as storage;

    #[path = "exfat.rs"]
    mod exfat;
    pub use exfat::ExFat;

    const _: () = assert!(core::mem::size_of::<ExFat<(), 4>>() < 2048);
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous embedded exFAT reader, over a
    //! `hadris_storage::local::BlockDevice`, whose futures need not be
    //! `Send`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_fat_raw::exfat::io::local as exio;
    use hadris_fat_raw::io::local as rawio;
    use hadris_storage::local as storage;

    #[path = "exfat.rs"]
    mod exfat;
    pub use exfat::ExFat;

    const _: () = assert!(core::mem::size_of::<ExFat<(), 4>>() < 2048);
}

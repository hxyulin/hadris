//! The embedded API: [`sync::Fat`] and `r#async::Fat`, a handle-based
//! FAT12, FAT16 and FAT32 driver for firmware without an allocator.
//!
//! `Fat<D, const FILES: usize = 4>` is built on the `hadris-fat-raw`
//! device primitives, not on `FatFs`. It keeps one 512-byte device block
//! in the struct, the volume's geometry, its [`Options`] and `FILES` file
//! slots, needs no allocator and never reads the device into a buffer
//! larger than 512 bytes. The device's blocks must be 512 bytes; FAT
//! sectors of 512 to 4096 bytes are read in 512-byte pieces.
//!
//! - Names are passed one component per call, relative to a [`Dir`], and
//!   compared with the fold of the [`Options`]: ASCII by default, so no
//!   Unicode case tables are linked, or
//!   `Options::new().with_fold(hadris_fat_raw::fold_unicode)` to compare
//!   as Windows and `FatFs` do.
//! - A [`File`] is a slot index, consumed by `close`. It carries a
//!   generation, so a stale handle or one from another volume fails with
//!   `InvalidHandle`. A dropped `File` keeps its slot until `unmount`;
//!   `sync` and `unmount` still write its size.
//! - `list` lends each [`Entry`] to a callback; its UTF-16 name lives on
//!   the call's stack. [`Entry::node`] with `open_node` opens a listed
//!   file without a second lookup.
//!
//! Formatting and checking are the shared, allocation-free
//! `hadris_fat::sync::{format, check}` and their `r#async` twins.
//!
//! `sync` takes a `hadris_storage::sync::BlockDevice`; `r#async` takes a
//! `hadris_storage::local::BlockDevice`, whose futures need not be `Send`,
//! for single-threaded executors such as Embassy.
//!
//! # Crash and cancellation safety
//!
//! Writes go through the same ordered primitives as `FatFs`: clusters are
//! marked in the FAT before anything points to them and freed after
//! nothing does, long names go before their short entry, and FAT copies
//! are written active copy first. The driver records the clusters an
//! unfinished operation holds and the long-name slots it was writing, so a
//! power cut, or a dropped `async` future, leaves at worst lost clusters,
//! and the next writing call or `sync` on the same `Fat` frees them and
//! clears the slots. A file's size is written to its entry by `flush`,
//! `close`, `sync` and `unmount`; until then an interruption leaves the
//! size the entry last recorded.

use hadris_fat_raw::ShortEntry;
use hadris_fat_raw::fold_ascii;
use hadris_fat_raw::io::{ChainPos, DirStart, Held};
use hadris_fs::{
    Attributes, CodePage, Cp437, DateTime, DateTimeError, DirCursor, FileType, Metadata, NoClock,
};

/// How the embedded API mounts a volume.
///
/// `Options::new()` mounts read-write, stamps entries with
/// 1980-01-01T00:00:00 (`hadris_fs::NoClock::TIME`), reads and writes
/// times as UTC, decodes short names in CP437 and folds ASCII letters
/// only when it compares names.
#[derive(Clone, Copy)]
pub struct Options {
    clock: fn() -> DateTime,
    fold: fn(u16) -> u16,
    utc_offset: Option<i16>,
    code_page: &'static dyn CodePage,
    read_only: bool,
}

impl core::fmt::Debug for Options {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Options")
            .field("utc_offset", &self.utc_offset)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

fn no_clock() -> DateTime {
    NoClock::TIME
}

impl Options {
    /// The defaults described above.
    pub const fn new() -> Self {
        Self {
            clock: no_clock,
            fold: fold_ascii,
            utc_offset: None,
            code_page: &Cp437,
            read_only: false,
        }
    }

    /// Mounts for reading only: the driver never writes to the device.
    pub const fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Stamps new and changed entries with the time `clock` returns.
    pub const fn with_clock(mut self, clock: fn() -> DateTime) -> Self {
        self.clock = clock;
        self
    }

    /// Compares names after folding each UTF-16 unit with `fold`, such as
    /// `hadris_fat_raw::fold_unicode`, which links the Unicode case
    /// tables. The fold also uppercases the non-ASCII characters of
    /// generated short names.
    pub const fn with_fold(mut self, fold: fn(u16) -> u16) -> Self {
        self.fold = fold;
        self
    }

    /// Reads and writes timestamps as local time `minutes` east of UTC.
    /// Fails when `minutes` is beyond a day.
    pub const fn with_utc_offset(mut self, minutes: i16) -> Result<Self, DateTimeError> {
        match NoClock::TIME.with_utc_offset_minutes(Some(minutes)) {
            Ok(_) => {
                self.utc_offset = Some(minutes);
                Ok(self)
            }
            Err(err) => Err(err),
        }
    }

    /// Reads and generates short names in `code_page`.
    pub const fn with_code_page(mut self, code_page: &'static dyn CodePage) -> Self {
        self.code_page = code_page;
        self
    }

    /// Whether the volume is mounted read-only.
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The clock that stamps entries.
    pub const fn clock(&self) -> fn() -> DateTime {
        self.clock
    }

    /// The fold names are compared with.
    pub const fn fold(&self) -> fn(u16) -> u16 {
        self.fold
    }

    /// The UTC offset of the volume's timestamps in minutes, `None` for
    /// UTC.
    pub const fn utc_offset(&self) -> Option<i16> {
        self.utc_offset
    }

    /// The code page of short names.
    pub const fn code_page(&self) -> &'static dyn CodePage {
        self.code_page
    }
}

/// A directory: the root or a subdirectory by its first cluster. `Copy`
/// and holds no slot; it stays valid while the directory exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dir {
    /// First cluster, 0 for the root.
    first: u32,
}

impl Dir {
    pub(crate) const ROOT: Self = Self { first: 0 };

    pub(crate) const fn new(first: u32) -> Self {
        Self { first }
    }

    pub(crate) const fn first(self) -> u32 {
        self.first
    }

    /// Whether this is the root directory.
    pub const fn is_root(self) -> bool {
        self.first == 0
    }
}

/// An open file: a slot of its volume and the generation of the open that
/// filled it. Not `Clone`, so `close` consumes the only handle.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a dropped File keeps its slot until unmount; close it"]
pub struct File {
    slot: u8,
    generation: u16,
}

impl File {
    pub(crate) const fn new(slot: u8, generation: u16) -> Self {
        Self { slot, generation }
    }

    pub(crate) const fn slot(&self) -> usize {
        self.slot as usize
    }

    pub(crate) const fn generation(&self) -> u16 {
        self.generation
    }
}

/// Where a listed entry is: valid for `open_node` until its directory
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Node {
    offset: u64,
}

impl Node {
    pub(crate) const fn new(offset: u64) -> Self {
        Self { offset }
    }

    pub(crate) const fn offset(self) -> u64 {
        self.offset
    }
}

/// One directory entry, lent to the callback of `list`.
#[derive(Debug)]
pub struct Entry<'a> {
    name: &'a [u16],
    short: ShortEntry,
    offset: u64,
    next: u64,
    zone: Option<i16>,
}

impl<'a> Entry<'a> {
    pub(crate) const fn new(
        name: &'a [u16],
        short: ShortEntry,
        offset: u64,
        next: u64,
        zone: Option<i16>,
    ) -> Self {
        Self {
            name,
            short,
            offset,
            next,
            zone,
        }
    }

    /// The name in UTF-16: the long name, or the short name decoded
    /// through the code page when there is none.
    pub fn name_utf16(&self) -> &'a [u16] {
        self.name
    }

    /// The characters of the name, with unpaired surrogates as U+FFFD.
    pub fn chars(&self) -> impl Iterator<Item = char> + 'a {
        hadris_fat_raw::name::utf16_chars(self.name.iter().copied())
    }

    /// The 11 bytes of the short name as stored.
    pub fn short_name(&self) -> [u8; 11] {
        self.short.name()
    }

    /// File or directory.
    pub fn file_type(&self) -> FileType {
        if self.short.is_dir() {
            FileType::Dir
        } else {
            FileType::File
        }
    }

    /// Whether the entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.short.is_dir()
    }

    /// The size the entry records; 0 for a directory. An open file's
    /// newer size is written by `flush`, `close` and `sync`.
    pub fn len(&self) -> u64 {
        if self.short.is_dir() {
            0
        } else {
            self.short.size() as u64
        }
    }

    /// Whether [`len`](Self::len) is 0.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The DOS attributes.
    pub fn attributes(&self) -> Attributes {
        crate::names::attributes(&self.short)
    }

    /// The metadata the entry stores.
    pub fn metadata(&self) -> Metadata {
        crate::names::metadata(&self.short, self.short.is_dir(), self.len(), self.zone)
    }

    /// The entry's position, for `open_node`.
    pub fn node(&self) -> Node {
        Node::new(self.offset)
    }

    /// Where a `list` continues after this entry.
    pub fn next_cursor(&self) -> DirCursor {
        DirCursor::from_raw(self.next)
    }
}

pub(crate) const FLAG_READ: u8 = 1;
pub(crate) const FLAG_WRITE: u8 = 1 << 1;
pub(crate) const FLAG_APPEND: u8 = 1 << 2;
/// The entry lacks the slot's size and modification time.
pub(crate) const FLAG_DIRTY: u8 = 1 << 3;

/// One open file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileSlot {
    /// Byte offset of the file's short entry; 0 when the slot is free.
    pub(crate) entry: u64,
    pub(crate) first: u32,
    pub(crate) size: u32,
    pub(crate) pos: u32,
    pub(crate) at: ChainPos,
    pub(crate) generation: u16,
    pub(crate) flags: u8,
}

impl FileSlot {
    pub(crate) const FREE: Self = Self {
        entry: 0,
        first: 0,
        size: 0,
        pos: 0,
        at: ChainPos::NONE,
        generation: 0,
        flags: 0,
    };

    pub(crate) const fn is_free(&self) -> bool {
        self.entry == 0
    }

    pub(crate) const fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// What links the chain of a [`Pending`] into the volume, and what
/// recovery does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// Nothing does: freed.
    None,
    /// The FAT entry of this cluster, a directory's last: kept when linked.
    Cluster(u32),
    /// The FAT entry of this cluster, a file's last before growth or a cut:
    /// the chain is cut there and freed.
    Tail(u32),
    /// The first cluster of the short entry at this offset: kept when
    /// linked.
    Entry(u64),
    /// The first cluster of the short entry at this offset, of an empty
    /// file being given its first clusters: cleared and freed.
    First(u64),
}

/// Clusters an unfinished operation holds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    pub(crate) held: Held,
    pub(crate) owner: Owner,
}

impl Pending {
    pub(crate) const NONE: Self = Self {
        held: Held::NONE,
        owner: Owner::None,
    };

    pub(crate) const fn chain(head: u32, owner: Owner) -> Self {
        Self {
            held: Held::new(head, 0),
            owner,
        }
    }

    pub(crate) const fn is_none(&self) -> bool {
        self.held.head() == 0 && self.held.extra() == 0
    }
}

/// Slots `first..=short` of `dir` being written or cleared: when slot
/// `short` holds no visible short entry, recovery clears the others.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run {
    pub(crate) dir: DirStart,
    pub(crate) first: u32,
    pub(crate) short: u32,
}

/// Mounts so far, which spread the generations of volumes in one program
/// apart. A load and a store rather than an atomic add, which targets
/// without compare-and-swap lack; a race only makes two bases equal.
static MOUNTS: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

/// The first generation of a volume with serial `serial`.
pub(crate) fn generation_base(serial: u32) -> u16 {
    use core::sync::atomic::Ordering::Relaxed;
    let seed = MOUNTS.load(Relaxed);
    MOUNTS.store(seed.wrapping_add(1), Relaxed);
    seed.wrapping_mul(4099) ^ serial as u16 ^ (serial >> 16) as u16
}

/// Deepest directory `remove_dir_all` descends to.
pub(crate) const MAX_DEPTH: u32 = 1024;
/// The device block size the embedded API takes.
pub(crate) const BLOCK: usize = 512;
/// Largest FAT file size.
pub(crate) const MAX_FILE_SIZE: u64 = u32::MAX as u64;

/// The short name of `entry` as UTF-16 in `out`, and its length.
pub(crate) fn short_units(
    entry: &ShortEntry,
    code_page: &dyn CodePage,
    out: &mut [u16; 24],
) -> usize {
    let mut text = [0u8; hadris_fat_raw::short_name::DISPLAY_MAX];
    let len = hadris_fat_raw::short_name::display(
        &entry.name(),
        entry.nt_case(),
        |byte| code_page.decode(byte),
        &mut text,
    );
    let mut count = 0;
    let text = core::str::from_utf8(&text[..len]).unwrap_or("");
    for unit in text.encode_utf16() {
        if count == out.len() {
            break;
        }
        out[count] = unit;
        count += 1;
    }
    count
}

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous embedded API, over a
    //! `hadris_storage::sync::BlockDevice`.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    /// Keeps a function out of line, to bound the caller's frame.
    macro_rules! outline {
        ($($item:tt)*) => { #[inline(never)] $($item)* };
    }

    use hadris_fat_raw::io::sync as rawio;
    use hadris_storage::sync as storage;

    #[path = "fat.rs"]
    mod fat;
    pub use fat::Fat;

    const _: () = assert!(core::mem::size_of::<Fat<(), 4>>() < 2048);
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous embedded API, over a
    //! `hadris_storage::local::BlockDevice`, whose futures need not be
    //! `Send`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    /// Leaves inlining to the compiler: an async function kept out of line
    /// returns its whole future into the caller's poll frame.
    macro_rules! outline {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_fat_raw::io::local as rawio;
    use hadris_storage::local as storage;

    #[path = "fat.rs"]
    mod fat;
    pub use fat::Fat;

    const _: () = assert!(core::mem::size_of::<Fat<(), 4>>() < 2048);
}

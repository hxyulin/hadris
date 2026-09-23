use hadris_fs::{Clock, FixedTable, NoClock, NodeTable};

use crate::code_page::Ascii;

/// How [`FatFs`](crate::sync::FatFs) mounts a volume: read-only or not, the
/// node table, the clock and the code page.
///
/// `MountOptions::new()` gives the defaults of `FatFs::open`: writable, a
/// `FixedTable<64>`, [`NoClock`] and [`Ascii`]. Each `with_*` method that
/// takes a value of another type changes the matching type parameter, and
/// `FatFs::open_with` takes the types from the options.
///
/// ```rust,no_run
/// # #[cfg(all(feature = "sync", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_fat::sync::FatFs;
/// use hadris_fat::{Cp437, MountOptions};
/// use hadris_fs::{HeapTable, SystemClock};
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let image = std::fs::read("disk.img")?;
/// let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
/// let options = MountOptions::new()
///     .with_table(HeapTable::new())
///     .with_clock(SystemClock)
///     .with_code_page(Cp437);
/// let fs = FatFs::open_with(dev, options)?;
/// assert!(!fs.is_read_only());
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "std")))]
/// # fn main() {}
/// ```
#[derive(Debug, Clone)]
pub struct MountOptions<T = FixedTable<64>, C = NoClock, P = Ascii> {
    pub(crate) read_only: bool,
    pub(crate) table: T,
    pub(crate) clock: C,
    pub(crate) code_page: P,
}

impl MountOptions {
    /// The defaults: writable, `FixedTable<64>`, [`NoClock`] and [`Ascii`].
    pub const fn new() -> Self {
        Self {
            read_only: false,
            table: FixedTable::new(),
            clock: NoClock,
            code_page: Ascii,
        }
    }
}

impl Default for MountOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, C, P> MountOptions<T, C, P> {
    /// Mounts for reading only when `read_only` is set: the driver never
    /// calls `write_blocks`, and writing methods fail with
    /// `ErrorKind::ReadOnly`.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Keeps pinned nodes in a table of the kind of `table`, such as
    /// `HeapTable::new()` or a larger `FixedTable::<N>::new()`. The driver
    /// makes its own table of that kind, so `table` holds no values.
    pub fn with_table<U: NodeTable<Value = ()>>(self, table: U) -> MountOptions<U, C, P> {
        MountOptions {
            read_only: self.read_only,
            table,
            clock: self.clock,
            code_page: self.code_page,
        }
    }

    /// Stamps new and modified entries with the time from `clock`.
    pub fn with_clock<K: Clock>(self, clock: K) -> MountOptions<T, K, P> {
        MountOptions {
            read_only: self.read_only,
            table: self.table,
            clock,
            code_page: self.code_page,
        }
    }

    /// Reads and generates short names in `code_page`.
    pub fn with_code_page<Q: crate::CodePage>(self, code_page: Q) -> MountOptions<T, C, Q> {
        MountOptions {
            read_only: self.read_only,
            table: self.table,
            clock: self.clock,
            code_page,
        }
    }

    /// Whether the volume is mounted for reading only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
}

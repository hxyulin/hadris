/// How a filesystem compares names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaseRule {
    /// Names that differ in case are different names.
    #[default]
    Sensitive,
    /// Lookups ignore case, but the stored name keeps the case it was created with.
    InsensitivePreserving,
    /// Lookups ignore case and the stored case is not meaningful.
    Insensitive,
}

/// Which names a filesystem can store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Charset {
    /// Arbitrary bytes, as on most Unix filesystems.
    #[default]
    Bytes,
    /// Unicode text, exchanged as UTF-8: FAT long names, exFAT, NTFS,
    /// Joliet and UDF.
    Unicode,
}

/// A metadata field a format may or may not store, for
/// [`Capabilities::stores`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Field {
    /// The creation time.
    Created,
    /// The modification time.
    Modified,
    /// The access time.
    Accessed,
    /// The metadata change time.
    Changed,
    /// POSIX permissions.
    Permissions,
    /// The owner.
    Owner,
    /// DOS-style attributes.
    Attributes,
    /// Device numbers.
    Device,
}

impl Field {
    const COUNT: usize = 8;

    const fn index(self) -> usize {
        match self {
            Self::Created => 0,
            Self::Modified => 1,
            Self::Accessed => 2,
            Self::Changed => 3,
            Self::Permissions => 4,
            Self::Owner => 5,
            Self::Attributes => 6,
            Self::Device => 7,
        }
    }
}

/// How much of a [`Field`] a format stores.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Stored {
    /// Not stored; `setattr` of it fails with
    /// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported).
    #[default]
    No,
    /// Stored in part, such as FAT's read-only bit standing for the write
    /// permission bits.
    Partial,
    /// Stored.
    Yes,
}

/// What a mounted filesystem supports.
///
/// Filesystems start from [`new`](Self::new), which describes a read-only
/// filesystem that stores no optional field, and enable what they support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Capabilities {
    writable: bool,
    symlinks: bool,
    hard_links: bool,
    case: CaseRule,
    charset: Charset,
    max_name_bytes: usize,
    stored: [Stored; Field::COUNT],
    timestamp_resolution_ns: u32,
}

impl Capabilities {
    /// A read-only filesystem that compares names by `case`, stores names
    /// in `charset` of up to `max_name_bytes` UTF-8 bytes (FUSE `namemax`),
    /// has one-second timestamps and stores no optional field.
    pub const fn new(case: CaseRule, charset: Charset, max_name_bytes: usize) -> Self {
        Self {
            writable: false,
            symlinks: false,
            hard_links: false,
            case,
            charset,
            max_name_bytes,
            stored: [Stored::No; Field::COUNT],
            timestamp_resolution_ns: 1_000_000_000,
        }
    }

    /// Returns whether the filesystem accepts writes.
    pub const fn writable(&self) -> bool {
        self.writable
    }

    /// Returns whether symbolic links are supported.
    pub const fn symlinks(&self) -> bool {
        self.symlinks
    }

    /// Returns whether hard links are supported.
    pub const fn hard_links(&self) -> bool {
        self.hard_links
    }

    /// Returns how names are compared.
    pub const fn case(&self) -> CaseRule {
        self.case
    }

    /// Returns which names the filesystem can store.
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// Returns the longest name the filesystem accepts, in UTF-8 bytes.
    pub const fn max_name_bytes(&self) -> usize {
        self.max_name_bytes
    }

    /// Returns how much of `field` the format stores.
    pub const fn stores(&self, field: Field) -> Stored {
        self.stored[field.index()]
    }

    /// Returns the resolution of modification times, in nanoseconds.
    pub const fn timestamp_resolution_ns(&self) -> u32 {
        self.timestamp_resolution_ns
    }

    /// Marks the filesystem as writable.
    pub const fn with_writable(self) -> Self {
        Self {
            writable: true,
            ..self
        }
    }

    /// Marks symbolic links as supported.
    pub const fn with_symlinks(self) -> Self {
        Self {
            symlinks: true,
            ..self
        }
    }

    /// Marks hard links as supported.
    pub const fn with_hard_links(self) -> Self {
        Self {
            hard_links: true,
            ..self
        }
    }

    /// Records how much of `field` the format stores.
    pub const fn with_stored(mut self, field: Field, stored: Stored) -> Self {
        self.stored[field.index()] = stored;
        self
    }

    /// Sets the resolution of modification times, in nanoseconds.
    pub const fn with_timestamp_resolution_ns(self, timestamp_resolution_ns: u32) -> Self {
        Self {
            timestamp_resolution_ns,
            ..self
        }
    }
}

/// Space usage of a filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FsStats {
    total_blocks: u64,
    free_blocks: u64,
    block_size: u32,
    file_count: Option<u64>,
}

impl FsStats {
    /// Creates statistics from block counts and the block size in bytes.
    pub const fn new(total_blocks: u64, free_blocks: u64, block_size: u32) -> Self {
        Self {
            total_blocks,
            free_blocks,
            block_size,
            file_count: None,
        }
    }

    /// Sets the number of files and directories.
    pub fn with_file_count(self, file_count: impl Into<Option<u64>>) -> Self {
        Self {
            file_count: file_count.into(),
            ..self
        }
    }

    /// Returns the total number of blocks.
    pub const fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    /// Returns the number of free blocks.
    pub const fn free_blocks(&self) -> u64 {
        self.free_blocks
    }

    /// Returns the number of used blocks.
    pub const fn used_blocks(&self) -> u64 {
        self.total_blocks.saturating_sub(self.free_blocks)
    }

    /// Returns the block size in bytes.
    pub const fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Returns the total size in bytes, saturating at `u64::MAX`.
    pub const fn total_bytes(&self) -> u64 {
        self.total_blocks.saturating_mul(self.block_size as u64)
    }

    /// Returns the free space in bytes, saturating at `u64::MAX`.
    pub const fn free_bytes(&self) -> u64 {
        self.free_blocks.saturating_mul(self.block_size as u64)
    }

    /// Returns the number of files and directories, if known.
    pub const fn file_count(&self) -> Option<u64> {
        self.file_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_start_read_only() {
        let caps = Capabilities::new(CaseRule::InsensitivePreserving, Charset::Unicode, 765);
        assert!(!caps.writable());
        assert_eq!(caps.stores(Field::Modified), Stored::No);
        let caps = caps
            .with_writable()
            .with_stored(Field::Modified, Stored::Yes)
            .with_stored(Field::Permissions, Stored::Partial)
            .with_timestamp_resolution_ns(2_000_000_000);
        assert!(caps.writable());
        assert!(!caps.symlinks() && !caps.hard_links());
        assert_eq!(caps.case(), CaseRule::InsensitivePreserving);
        assert_eq!(caps.charset(), Charset::Unicode);
        assert_eq!(caps.max_name_bytes(), 765);
        assert_eq!(caps.stores(Field::Modified), Stored::Yes);
        assert_eq!(caps.stores(Field::Permissions), Stored::Partial);
        assert_eq!(caps.stores(Field::Owner), Stored::No);
        assert_eq!(caps.timestamp_resolution_ns(), 2_000_000_000);
    }

    #[test]
    fn stats_saturate() {
        let stats = FsStats::new(10, 20, 512).with_file_count(3);
        assert_eq!(stats.used_blocks(), 0);
        assert_eq!(stats.total_bytes(), 5120);
        assert_eq!(stats.file_count(), Some(3));
        assert_eq!(FsStats::new(u64::MAX, 0, 4096).total_bytes(), u64::MAX);
    }
}

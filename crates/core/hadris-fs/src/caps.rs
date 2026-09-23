/// How a filesystem compares names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaseSensitivity {
    /// Names that differ in case are different names.
    #[default]
    Sensitive,
    /// Lookups ignore case, but the stored name keeps the case it was created with.
    InsensitivePreserving,
    /// Lookups ignore case and the stored case is not meaningful.
    Insensitive,
}

/// The encoding of names on disk, which determines which bytes a name may hold.
///
/// Whether this is precise enough for a VFS to translate names, or whether
/// formats also need a name codec, is open question Q6 in
/// `docs/v3-api-design.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameCharset {
    /// Arbitrary bytes, as on most Unix filesystems.
    #[default]
    Bytes,
    /// UTF-8 text.
    Utf8,
    /// UCS-2, as in Joliet. Names are exchanged as UTF-8.
    Ucs2,
    /// UTF-16, as in FAT long names, exFAT and NTFS. Names are exchanged as UTF-8.
    Utf16,
    /// ISO 9660 d-characters: `A`-`Z`, `0`-`9` and `_`.
    DCharacters,
    /// An OEM code page, as in FAT short names.
    OemCodePage,
}

/// What a mounted filesystem supports.
///
/// Filesystems start from [`new`](Self::new), which describes a read-only
/// filesystem with no optional features, and enable what they support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Capabilities {
    writable: bool,
    symlinks: bool,
    hard_links: bool,
    permissions: bool,
    owners: bool,
    case_sensitivity: CaseSensitivity,
    max_name_len: usize,
    name_charset: NameCharset,
    timestamp_resolution_ns: u32,
}

impl Capabilities {
    /// A read-only, case-sensitive filesystem with byte names of up to 255
    /// bytes, one-second timestamps and no optional features.
    pub const fn new() -> Self {
        Self {
            writable: false,
            symlinks: false,
            hard_links: false,
            permissions: false,
            owners: false,
            case_sensitivity: CaseSensitivity::Sensitive,
            max_name_len: 255,
            name_charset: NameCharset::Bytes,
            timestamp_resolution_ns: 1_000_000_000,
        }
    }

    /// Returns whether the filesystem accepts writes.
    pub const fn is_writable(&self) -> bool {
        self.writable
    }

    /// Returns whether symbolic links are supported.
    pub const fn supports_symlinks(&self) -> bool {
        self.symlinks
    }

    /// Returns whether hard links are supported.
    pub const fn supports_hard_links(&self) -> bool {
        self.hard_links
    }

    /// Returns whether POSIX permissions are stored.
    pub const fn supports_permissions(&self) -> bool {
        self.permissions
    }

    /// Returns whether owner user and group IDs are stored.
    pub const fn supports_owners(&self) -> bool {
        self.owners
    }

    /// Returns how names are compared.
    pub const fn case_sensitivity(&self) -> CaseSensitivity {
        self.case_sensitivity
    }

    /// Returns the longest name the filesystem accepts, in bytes of the
    /// exchanged (UTF-8 or raw) form.
    pub const fn max_name_len(&self) -> usize {
        self.max_name_len
    }

    /// Returns the on-disk name encoding.
    pub const fn name_charset(&self) -> NameCharset {
        self.name_charset
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

    /// Marks POSIX permissions as stored.
    pub const fn with_permissions(self) -> Self {
        Self {
            permissions: true,
            ..self
        }
    }

    /// Marks owner IDs as stored.
    pub const fn with_owners(self) -> Self {
        Self {
            owners: true,
            ..self
        }
    }

    /// Sets how names are compared.
    pub const fn with_case_sensitivity(self, case_sensitivity: CaseSensitivity) -> Self {
        Self {
            case_sensitivity,
            ..self
        }
    }

    /// Sets the longest accepted name in bytes.
    pub const fn with_max_name_len(self, max_name_len: usize) -> Self {
        Self {
            max_name_len,
            ..self
        }
    }

    /// Sets the on-disk name encoding.
    pub const fn with_name_charset(self, name_charset: NameCharset) -> Self {
        Self {
            name_charset,
            ..self
        }
    }

    /// Sets the resolution of modification times, in nanoseconds.
    pub const fn with_timestamp_resolution_ns(self, timestamp_resolution_ns: u32) -> Self {
        Self {
            timestamp_resolution_ns,
            ..self
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::new()
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
        let caps = Capabilities::default();
        assert!(!caps.is_writable());
        let caps = caps
            .with_writable()
            .with_case_sensitivity(CaseSensitivity::InsensitivePreserving)
            .with_name_charset(NameCharset::Utf16)
            .with_timestamp_resolution_ns(2_000_000_000);
        assert!(caps.is_writable());
        assert!(!caps.supports_symlinks());
        assert_eq!(caps.name_charset(), NameCharset::Utf16);
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

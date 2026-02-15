//! Basic extent and file metadata types for filesystem operations.
//!
//! These types are no-std compatible and don't require allocation.

/// A contiguous region on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
#[repr(C)]
pub struct Extent {
    /// Starting logical sector (LBA).
    pub sector: u32,
    /// Padding for alignment.
    _padding: u32,
    /// Size in bytes.
    pub length: u64,
}

impl Extent {
    /// Creates a new extent.
    #[inline]
    pub const fn new(sector: u32, length: u64) -> Self {
        Self {
            sector,
            _padding: 0,
            length,
        }
    }

    /// Returns the end sector (exclusive) based on the given sector size.
    #[inline]
    pub const fn end_sector(&self, sector_size: u32) -> u32 {
        let sectors = (self.length + sector_size as u64 - 1) / sector_size as u64;
        self.sector + sectors as u32
    }

    /// Returns the number of sectors this extent spans.
    #[inline]
    pub const fn sector_count(&self, sector_size: u32) -> u32 {
        ((self.length + sector_size as u64 - 1) / sector_size as u64) as u32
    }

    /// Checks if this extent overlaps with another.
    #[inline]
    pub const fn overlaps(&self, other: &Extent, sector_size: u32) -> bool {
        let self_end = self.end_sector(sector_size);
        let other_end = other.end_sector(sector_size);
        self.sector < other_end && other.sector < self_end
    }

    /// Checks if this extent is empty (zero length).
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
}

/// File type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum FileType {
    /// A regular file containing data.
    #[default]
    RegularFile = 0,
    /// A directory containing other entries.
    Directory = 1,
    /// A symbolic link to another file or directory.
    Symlink = 2,
}

impl FileType {
    /// Returns true if this is a directory.
    #[inline]
    pub const fn is_directory(&self) -> bool {
        matches!(self, FileType::Directory)
    }

    /// Returns true if this is a regular file.
    #[inline]
    pub const fn is_file(&self) -> bool {
        matches!(self, FileType::RegularFile)
    }

    /// Returns true if this is a symbolic link.
    #[inline]
    pub const fn is_symlink(&self) -> bool {
        matches!(self, FileType::Symlink)
    }
}

/// Generic timestamps using Unix epoch (seconds since 1970-01-01 00:00:00 UTC).
///
/// This representation is no-std compatible and can be converted to
/// platform-specific types when needed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
#[repr(C)]
pub struct Timestamps {
    /// Creation time (seconds since Unix epoch).
    pub created: u64,
    /// Last modification time (seconds since Unix epoch).
    pub modified: u64,
    /// Last access time (seconds since Unix epoch).
    pub accessed: u64,
}

impl Timestamps {
    /// Creates new timestamps with all fields set to the same value.
    #[inline]
    pub const fn new(time: u64) -> Self {
        Self {
            created: time,
            modified: time,
            accessed: time,
        }
    }

    /// Creates timestamps with explicit values for each field.
    #[inline]
    pub const fn with_times(created: u64, modified: u64, accessed: u64) -> Self {
        Self {
            created,
            modified,
            accessed,
        }
    }

    /// Returns the most recent timestamp.
    #[inline]
    pub const fn most_recent(&self) -> u64 {
        let max = if self.created > self.modified {
            self.created
        } else {
            self.modified
        };
        if self.accessed > max {
            self.accessed
        } else {
            max
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extent_basics() {
        let extent = Extent::new(100, 4096);
        assert_eq!(extent.sector, 100);
        assert_eq!(extent.length, 4096);
        assert!(!extent.is_empty());

        // With 2048-byte sectors, 4096 bytes = 2 sectors
        assert_eq!(extent.sector_count(2048), 2);
        assert_eq!(extent.end_sector(2048), 102);
    }

    #[test]
    fn test_extent_overlap() {
        let a = Extent::new(100, 4096); // sectors 100-101
        let b = Extent::new(101, 2048); // sector 101
        let c = Extent::new(102, 2048); // sector 102

        assert!(a.overlaps(&b, 2048));
        assert!(!a.overlaps(&c, 2048));
        assert!(!b.overlaps(&c, 2048));
    }

    #[test]
    fn test_file_type() {
        assert!(FileType::Directory.is_directory());
        assert!(FileType::RegularFile.is_file());
        assert!(FileType::Symlink.is_symlink());
    }

    #[test]
    fn test_timestamps() {
        let ts = Timestamps::new(1000);
        assert_eq!(ts.created, 1000);
        assert_eq!(ts.modified, 1000);
        assert_eq!(ts.accessed, 1000);

        let ts2 = Timestamps::with_times(100, 200, 150);
        assert_eq!(ts2.most_recent(), 200);
    }
}

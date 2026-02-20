//! Optical media types for CD/DVD/Blu-ray operations.
//!
//! This module contains types specific to optical media filesystems
//! like ISO 9660 and UDF. It is gated behind the `optical` feature.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use super::types::layout::DirectoryLayout;

/// Standard sector size for optical media (CD/DVD/Blu-ray).
pub const OPTICAL_SECTOR_SIZE: usize = 2048;

/// Multi-session information for optical media.
///
/// On optical media (CD/DVD), multiple "sessions" can be written
/// sequentially. Each session has its own complete volume descriptor set,
/// and the latest session's metadata references all visible files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionInfo {
    /// Session number (1-based).
    pub session_number: u16,
    /// First sector of this session.
    pub start_sector: u32,
    /// Last sector of this session (inclusive).
    pub end_sector: u32,
    /// Whether this is the last (most recent) session.
    pub is_last_session: bool,
}

impl SessionInfo {
    /// Creates a new session info.
    pub const fn new(session_number: u16, start_sector: u32, end_sector: u32) -> Self {
        Self {
            session_number,
            start_sector,
            end_sector,
            is_last_session: false,
        }
    }

    /// Returns the size of this session in sectors.
    #[inline]
    pub const fn sector_count(&self) -> u32 {
        self.end_sector - self.start_sector + 1
    }

    /// Returns the size of this session in bytes.
    #[inline]
    pub const fn byte_size(&self) -> u64 {
        self.sector_count() as u64 * OPTICAL_SECTOR_SIZE as u64
    }

    /// Marks this session as the last session.
    pub const fn as_last(mut self) -> Self {
        self.is_last_session = true;
        self
    }
}

impl core::fmt::Display for SessionInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "session {} (sectors {}-{})",
            self.session_number, self.start_sector, self.end_sector
        )
    }
}

impl Default for SessionInfo {
    fn default() -> Self {
        Self {
            session_number: 1,
            start_sector: 0,
            end_sector: 0,
            is_last_session: true,
        }
    }
}

/// Trait for optical media metadata writers.
///
/// Implement this trait for filesystems that can write metadata
/// for pre-laid-out files. The metadata writer receives a directory
/// tree with extents already calculated, and writes only the
/// filesystem metadata (volume descriptors, directory records, etc.)
/// without writing any file data.
#[cfg(feature = "alloc")]
pub trait OpticalMetadataWriter {
    /// Options type for the writer.
    type Options;
    /// Error type for the writer.
    type Error;

    /// Write metadata for the given directory tree.
    ///
    /// The `root` contains files with pre-calculated extents.
    /// This method should write:
    /// - Volume descriptors
    /// - Directory records / file identifiers
    /// - Path tables (for ISO 9660)
    /// - Allocation descriptors (for UDF)
    ///
    /// It should NOT write file data, as that is assumed to
    /// already exist at the locations specified in the extents.
    fn write_metadata<W: hadris_io::Write + hadris_io::Seek>(
        writer: &mut W,
        root: &DirectoryLayout,
        options: &Self::Options,
    ) -> Result<(), Self::Error>;
}

/// Media type classification for optical media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum OpticalMediaType {
    /// CD-ROM (read-only, up to ~700 MB).
    #[default]
    CdRom = 0,
    /// CD-R (write-once, up to ~700 MB).
    CdR = 1,
    /// CD-RW (rewritable, up to ~700 MB).
    CdRw = 2,
    /// DVD-ROM (read-only, 4.7 GB or 8.5 GB).
    DvdRom = 3,
    /// DVD-R (write-once, 4.7 GB).
    DvdR = 4,
    /// DVD+R (write-once, 4.7 GB).
    DvdPlusR = 5,
    /// DVD-RW (rewritable, 4.7 GB).
    DvdRw = 6,
    /// DVD+RW (rewritable, 4.7 GB).
    DvdPlusRw = 7,
    /// DVD-RAM (random access, 4.7 GB).
    DvdRam = 8,
    /// BD-ROM (read-only Blu-ray, 25/50 GB).
    BdRom = 9,
    /// BD-R (write-once Blu-ray, 25/50 GB).
    BdR = 10,
    /// BD-RE (rewritable Blu-ray, 25/50 GB).
    BdRe = 11,
}

impl core::fmt::Display for OpticalMediaType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CdRom => write!(f, "CD-ROM"),
            Self::CdR => write!(f, "CD-R"),
            Self::CdRw => write!(f, "CD-RW"),
            Self::DvdRom => write!(f, "DVD-ROM"),
            Self::DvdR => write!(f, "DVD-R"),
            Self::DvdPlusR => write!(f, "DVD+R"),
            Self::DvdRw => write!(f, "DVD-RW"),
            Self::DvdPlusRw => write!(f, "DVD+RW"),
            Self::DvdRam => write!(f, "DVD-RAM"),
            Self::BdRom => write!(f, "BD-ROM"),
            Self::BdR => write!(f, "BD-R"),
            Self::BdRe => write!(f, "BD-RE"),
        }
    }
}

impl OpticalMediaType {
    /// Returns true if this media type is writable (write-once or rewritable).
    #[inline]
    pub const fn is_writable(&self) -> bool {
        !matches!(self, Self::CdRom | Self::DvdRom | Self::BdRom)
    }

    /// Returns true if this media type is rewritable.
    #[inline]
    pub const fn is_rewritable(&self) -> bool {
        matches!(
            self,
            Self::CdRw | Self::DvdRw | Self::DvdPlusRw | Self::DvdRam | Self::BdRe
        )
    }

    /// Returns true if this media type is write-once.
    #[inline]
    pub const fn is_write_once(&self) -> bool {
        matches!(self, Self::CdR | Self::DvdR | Self::DvdPlusR | Self::BdR)
    }

    /// Returns the approximate capacity in bytes.
    #[inline]
    pub const fn capacity(&self) -> u64 {
        match self {
            Self::CdRom | Self::CdR | Self::CdRw => 700 * 1024 * 1024,
            Self::DvdRom
            | Self::DvdR
            | Self::DvdPlusR
            | Self::DvdRw
            | Self::DvdPlusRw
            | Self::DvdRam => 4_700_000_000,
            Self::BdRom | Self::BdR | Self::BdRe => 25_000_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_info() {
        let session = SessionInfo::new(1, 0, 999).as_last();
        assert_eq!(session.session_number, 1);
        assert_eq!(session.sector_count(), 1000);
        assert_eq!(session.byte_size(), 1000 * 2048);
        assert!(session.is_last_session);
    }

    #[test]
    fn test_media_type() {
        assert!(!OpticalMediaType::CdRom.is_writable());
        assert!(OpticalMediaType::CdR.is_writable());
        assert!(OpticalMediaType::CdR.is_write_once());
        assert!(!OpticalMediaType::CdR.is_rewritable());
        assert!(OpticalMediaType::CdRw.is_rewritable());
        assert!(!OpticalMediaType::CdRw.is_write_once());
    }
}

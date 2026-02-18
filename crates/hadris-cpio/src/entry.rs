use crate::error::Result;
use crate::header::RawNewcHeader;
use crate::mode::FileType;

/// Decoded CPIO entry header with all fields parsed from hex to `u32`.
///
/// Each field corresponds to a field in the 110-byte newc ASCII header.
/// See [`RawNewcHeader`] for the raw on-disk layout.
#[derive(Debug, Clone)]
pub struct CpioEntryHeader {
    /// Inode number.
    pub ino: u32,
    /// File mode (file type + permissions). Use [`FileType::from_mode`] to extract the type.
    pub mode: u32,
    /// Owner user ID.
    pub uid: u32,
    /// Owner group ID.
    pub gid: u32,
    /// Number of hard links.
    pub nlink: u32,
    /// Modification time (seconds since Unix epoch).
    pub mtime: u32,
    /// File data size in bytes.
    pub filesize: u32,
    /// Major number of the device containing this file.
    pub devmajor: u32,
    /// Minor number of the device containing this file.
    pub devminor: u32,
    /// Major number of the device (for device nodes).
    pub rdevmajor: u32,
    /// Minor number of the device (for device nodes).
    pub rdevminor: u32,
    /// CRC checksum (only meaningful in `070702` format).
    pub check: u32,
}

impl CpioEntryHeader {
    /// Parse a decoded header from a [`RawNewcHeader`].
    pub fn from_raw(raw: &RawNewcHeader) -> Result<Self> {
        Ok(Self {
            ino: raw.ino()?,
            mode: raw.mode()?,
            uid: raw.uid()?,
            gid: raw.gid()?,
            nlink: raw.nlink()?,
            mtime: raw.mtime()?,
            filesize: raw.filesize()?,
            devmajor: raw.devmajor()?,
            devminor: raw.devminor()?,
            rdevmajor: raw.rdevmajor()?,
            rdevminor: raw.rdevminor()?,
            check: raw.check()?,
        })
    }

    /// Returns the file type extracted from the mode bits.
    pub fn file_type(&self) -> FileType {
        FileType::from_mode(self.mode)
    }

    /// Returns the lower 12 bits of the mode (Unix permission bits).
    pub fn permissions(&self) -> u32 {
        self.mode & 0o7777
    }

    /// Returns true if this looks like a TRAILER sentinel (ino=0, mode=0, nlink=1, filesize=0).
    pub fn is_trailer_like(&self) -> bool {
        self.ino == 0 && self.mode == 0 && self.nlink == 1 && self.filesize == 0
    }

    /// Returns true if this is a hard link (regular file with nlink > 1 and filesize == 0).
    /// The actual data is stored only in the last link entry (with non-zero filesize).
    pub fn is_hard_link(&self) -> bool {
        self.file_type() == FileType::Regular && self.nlink > 1 && self.filesize == 0
    }
}

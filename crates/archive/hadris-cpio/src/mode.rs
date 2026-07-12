use core::fmt;

/// Bitmask for extracting the file type from a Unix mode value.
const S_IFMT: u32 = 0o170000;
const S_IFSOCK: u32 = 0o140000;
const S_IFLNK: u32 = 0o120000;
const S_IFREG: u32 = 0o100000;
const S_IFBLK: u32 = 0o060000;
const S_IFDIR: u32 = 0o040000;
const S_IFCHR: u32 = 0o020000;
const S_IFIFO: u32 = 0o010000;

/// Unix file type extracted from the upper bits of a CPIO mode field.
///
/// The file type occupies bits 12-15 of the mode value (`mode & 0o170000`).
/// Use [`FileType::from_mode`] to extract the type from a raw mode value,
/// and [`make_mode`] to combine a file type with permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Socket,
    Symlink,
    Regular,
    BlockDevice,
    Directory,
    CharDevice,
    Fifo,
    Unknown(u32),
}

impl FileType {
    /// Extract the file type from a raw Unix mode value.
    pub fn from_mode(mode: u32) -> Self {
        match mode & S_IFMT {
            S_IFSOCK => FileType::Socket,
            S_IFLNK => FileType::Symlink,
            S_IFREG => FileType::Regular,
            S_IFBLK => FileType::BlockDevice,
            S_IFDIR => FileType::Directory,
            S_IFCHR => FileType::CharDevice,
            S_IFIFO => FileType::Fifo,
            other => FileType::Unknown(other),
        }
    }

    /// Convert this file type back to the upper mode bits.
    pub fn to_mode_bits(self) -> u32 {
        match self {
            FileType::Socket => S_IFSOCK,
            FileType::Symlink => S_IFLNK,
            FileType::Regular => S_IFREG,
            FileType::BlockDevice => S_IFBLK,
            FileType::Directory => S_IFDIR,
            FileType::CharDevice => S_IFCHR,
            FileType::Fifo => S_IFIFO,
            FileType::Unknown(v) => v,
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileType::Socket => write!(f, "socket"),
            FileType::Symlink => write!(f, "symlink"),
            FileType::Regular => write!(f, "regular file"),
            FileType::BlockDevice => write!(f, "block device"),
            FileType::Directory => write!(f, "directory"),
            FileType::CharDevice => write!(f, "char device"),
            FileType::Fifo => write!(f, "fifo"),
            FileType::Unknown(v) => write!(f, "unknown({v:#o})"),
        }
    }
}

/// Combine a [`FileType`] and permission bits into a complete mode value.
///
/// The permission bits are masked to the lower 12 bits (`0o7777`).
pub fn make_mode(file_type: FileType, permissions: u32) -> u32 {
    file_type.to_mode_bits() | (permissions & 0o7777)
}

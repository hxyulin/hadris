use hadris_fs::{DeviceNumber, FileType};

use crate::options::Format;
use crate::raw::{
    self, BINARY_HEADER_LEN, BinaryHeader, NEWC_HEADER_LEN, NewcHeader, ODC_HEADER_LEN, OdcHeader,
};

/// A decoded header, whatever the format.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    pub format: Format,
    pub ino: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub mtime: u64,
    pub len: u64,
    pub dev: DeviceNumber,
    pub rdev: DeviceNumber,
    pub namesize: usize,
    pub check: u32,
}

impl Header {
    pub const EMPTY: Self = Self {
        format: Format::Newc,
        ino: 0,
        mode: 0,
        uid: 0,
        gid: 0,
        nlink: 0,
        mtime: 0,
        len: 0,
        dev: DeviceNumber::new(0, 0),
        rdev: DeviceNumber::new(0, 0),
        namesize: 0,
        check: 0,
    };
}

/// The file type the mode bits name, or `None` for unknown bits.
pub(crate) fn file_type(mode: u32) -> Option<FileType> {
    Some(match mode & raw::S_IFMT {
        raw::S_IFREG => FileType::File,
        raw::S_IFDIR => FileType::Dir,
        raw::S_IFLNK => FileType::Symlink,
        raw::S_IFCHR => FileType::CharDevice,
        raw::S_IFBLK => FileType::BlockDevice,
        raw::S_IFIFO => FileType::Fifo,
        raw::S_IFSOCK => FileType::Socket,
        _ => return None,
    })
}

/// Splits a 16-bit style device number of the old formats.
pub(crate) fn split_dev(dev: u32) -> DeviceNumber {
    DeviceNumber::new(dev >> 8, dev & 0xFF)
}

/// Joins a device number into the old formats' `major << 8 | minor`.
pub(crate) fn join_dev(dev: DeviceNumber) -> Option<u32> {
    (dev.minor() <= 0xFF)
        .then(|| dev.major().checked_mul(256))
        .flatten()
        .map(|major| major | dev.minor())
}

/// Bytes still to read after the first six to complete a header.
pub(crate) fn header_len(format: Format) -> usize {
    match format {
        Format::Odc => ODC_HEADER_LEN,
        Format::Binary => BINARY_HEADER_LEN,
        _ => NEWC_HEADER_LEN,
    }
}

/// The format a header starting with `start` is in.
pub(crate) fn detect(start: &[u8; 6]) -> Option<Format> {
    if BinaryHeader::is_magic([start[0], start[1]]) {
        return Some(Format::Binary);
    }
    match start {
        s if s == raw::NEWC_MAGIC => Some(Format::Newc),
        s if s == raw::NEWC_CRC_MAGIC => Some(Format::Crc),
        s if s == raw::ODC_MAGIC => Some(Format::Odc),
        _ => None,
    }
}

/// Decodes a complete header of `format` from `bytes`.
pub(crate) fn decode(format: Format, bytes: &[u8]) -> Option<Header> {
    match format {
        Format::Odc => {
            let fields = OdcHeader(bytes.try_into().ok()?).fields()?;
            Some(Header {
                format,
                ino: u64::from(fields.ino),
                mode: fields.mode,
                uid: fields.uid,
                gid: fields.gid,
                nlink: fields.nlink,
                mtime: fields.mtime,
                len: fields.filesize,
                dev: split_dev(fields.dev),
                rdev: split_dev(fields.rdev),
                namesize: fields.namesize as usize,
                check: 0,
            })
        }
        Format::Binary => {
            let fields = BinaryHeader(bytes.try_into().ok()?).fields()?;
            Some(Header {
                format,
                ino: u64::from(fields.ino),
                mode: u32::from(fields.mode),
                uid: u32::from(fields.uid),
                gid: u32::from(fields.gid),
                nlink: u32::from(fields.nlink),
                mtime: u64::from(fields.mtime),
                len: u64::from(fields.filesize),
                dev: split_dev(u32::from(fields.dev)),
                rdev: split_dev(u32::from(fields.rdev)),
                namesize: usize::from(fields.namesize),
                check: 0,
            })
        }
        _ => {
            let fields = NewcHeader(bytes.try_into().ok()?).fields()?;
            Some(Header {
                format,
                ino: u64::from(fields.ino),
                mode: fields.mode,
                uid: fields.uid,
                gid: fields.gid,
                nlink: fields.nlink,
                mtime: u64::from(fields.mtime),
                len: u64::from(fields.filesize),
                dev: DeviceNumber::new(fields.devmajor, fields.devminor),
                rdev: DeviceNumber::new(fields.rdevmajor, fields.rdevminor),
                namesize: fields.namesize as usize,
                check: fields.check,
            })
        }
    }
}

/// Padding after the header and name of an entry.
pub(crate) fn name_padding(format: Format, namesize: usize) -> usize {
    match format {
        Format::Odc => 0,
        Format::Binary => namesize % 2,
        _ => (4 - (NEWC_HEADER_LEN + namesize) % 4) % 4,
    }
}

/// Padding after the data of an entry.
pub(crate) fn data_padding(format: Format, len: u64) -> usize {
    match format {
        Format::Odc => 0,
        Format::Binary => (len % 2) as usize,
        _ => ((4 - len % 4) % 4) as usize,
    }
}

/// The byte sum a `070702` check field holds.
pub(crate) fn checksum(sum: u32, data: &[u8]) -> u32 {
    data.iter()
        .fold(sum, |sum, byte| sum.wrapping_add(u32::from(*byte)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers_of_the_old_formats() {
        assert_eq!(split_dev(0x0501), DeviceNumber::new(5, 1));
        assert_eq!(join_dev(DeviceNumber::new(5, 1)), Some(0x0501));
        assert_eq!(join_dev(DeviceNumber::new(5, 256)), None);
        assert_eq!(join_dev(DeviceNumber::new(u32::MAX, 0)), None);
    }

    #[test]
    fn padding_follows_each_format() {
        assert_eq!(name_padding(Format::Newc, 2), 0);
        assert_eq!(name_padding(Format::Newc, 3), 3);
        assert_eq!(data_padding(Format::Crc, 5), 3);
        assert_eq!(name_padding(Format::Binary, 3), 1);
        assert_eq!(data_padding(Format::Odc, 5), 0);
    }
}

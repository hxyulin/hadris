//! On-disk cpio header layouts and constants.
//!
//! Mirrors the formats as `cpio(5)` and the Linux initramfs documentation
//! describe them; may gain items, existing items follow the formats. The
//! reader and writer use these layouts; tools that inspect archives byte by
//! byte can use them directly.

/// Magic of the portable ASCII format (`newc`).
pub const NEWC_MAGIC: &[u8; 6] = b"070701";
/// Magic of `newc` with a per-file checksum (`crc`).
pub const NEWC_CRC_MAGIC: &[u8; 6] = b"070702";
/// Magic of the old portable ASCII format (`odc`).
pub const ODC_MAGIC: &[u8; 6] = b"070707";
/// Magic of the old binary format, as a 16-bit number in the archive's
/// byte order.
pub const BINARY_MAGIC: u16 = 0o070707;

/// Length of a `newc` header.
pub const NEWC_HEADER_LEN: usize = 110;
/// Length of an `odc` header.
pub const ODC_HEADER_LEN: usize = 76;
/// Length of an old binary header.
pub const BINARY_HEADER_LEN: usize = 26;

/// The name of the entry that ends an archive.
pub const TRAILER_NAME: &[u8] = b"TRAILER!!!";
/// The longest name, with its terminating NUL, the reader and writer accept.
pub const PATH_MAX: usize = 4096;

/// Mask of the file type bits of a mode.
pub const S_IFMT: u32 = 0o170000;
/// Socket.
pub const S_IFSOCK: u32 = 0o140000;
/// Symbolic link.
pub const S_IFLNK: u32 = 0o120000;
/// Regular file.
pub const S_IFREG: u32 = 0o100000;
/// Block device.
pub const S_IFBLK: u32 = 0o060000;
/// Directory.
pub const S_IFDIR: u32 = 0o040000;
/// Character device.
pub const S_IFCHR: u32 = 0o020000;
/// Named pipe.
pub const S_IFIFO: u32 = 0o010000;

/// The fields of a `newc` header, each eight hexadecimal digits on disk.
///
/// `namesize` counts the terminating NUL. `check` is the byte sum of the
/// data for `070702` and zero for `070701`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NewcFields {
    /// Inode number.
    pub ino: u32,
    /// File type and permission bits.
    pub mode: u32,
    /// Owner user id.
    pub uid: u32,
    /// Owner group id.
    pub gid: u32,
    /// Number of links.
    pub nlink: u32,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: u32,
    /// Length of the data.
    pub filesize: u32,
    /// Major number of the device holding the file.
    pub devmajor: u32,
    /// Minor number of the device holding the file.
    pub devminor: u32,
    /// Major number of a device node.
    pub rdevmajor: u32,
    /// Minor number of a device node.
    pub rdevminor: u32,
    /// Length of the name with its NUL.
    pub namesize: u32,
    /// Checksum of the data, `070702` only.
    pub check: u32,
}

/// A `newc` header: magic and thirteen hexadecimal fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NewcHeader(pub [u8; NEWC_HEADER_LEN]);

impl NewcHeader {
    /// Builds a header. `crc` selects the `070702` magic.
    pub fn new(crc: bool, fields: &NewcFields) -> Self {
        let mut data = [0u8; NEWC_HEADER_LEN];
        data[..6].copy_from_slice(if crc { NEWC_CRC_MAGIC } else { NEWC_MAGIC });
        let values = [
            fields.ino,
            fields.mode,
            fields.uid,
            fields.gid,
            fields.nlink,
            fields.mtime,
            fields.filesize,
            fields.devmajor,
            fields.devminor,
            fields.rdevmajor,
            fields.rdevminor,
            fields.namesize,
            fields.check,
        ];
        for (index, value) in values.iter().enumerate() {
            let start = 6 + index * 8;
            write_hex(*value, &mut data[start..start + 8]);
        }
        Self(data)
    }

    /// Whether the magic is `070702`.
    pub fn is_crc(&self) -> bool {
        &self.0[..6] == NEWC_CRC_MAGIC
    }

    /// The fields, or `None` when the magic is not `070701` or `070702` or a
    /// field is not hexadecimal.
    pub fn fields(&self) -> Option<NewcFields> {
        if &self.0[..6] != NEWC_MAGIC && &self.0[..6] != NEWC_CRC_MAGIC {
            return None;
        }
        let field = |index: usize| parse_hex(&self.0[6 + index * 8..14 + index * 8]);
        Some(NewcFields {
            ino: field(0)?,
            mode: field(1)?,
            uid: field(2)?,
            gid: field(3)?,
            nlink: field(4)?,
            mtime: field(5)?,
            filesize: field(6)?,
            devmajor: field(7)?,
            devminor: field(8)?,
            rdevmajor: field(9)?,
            rdevminor: field(10)?,
            namesize: field(11)?,
            check: field(12)?,
        })
    }
}

/// The fields of an `odc` header, octal digits on disk.
///
/// `dev` and `rdev` are six digits, `mtime` and `filesize` eleven, the rest
/// six. `namesize` counts the terminating NUL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OdcFields {
    /// Device holding the file.
    pub dev: u32,
    /// Inode number.
    pub ino: u32,
    /// File type and permission bits.
    pub mode: u32,
    /// Owner user id.
    pub uid: u32,
    /// Owner group id.
    pub gid: u32,
    /// Number of links.
    pub nlink: u32,
    /// Device number of a device node.
    pub rdev: u32,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: u64,
    /// Length of the name with its NUL.
    pub namesize: u32,
    /// Length of the data.
    pub filesize: u64,
}

/// Width in octal digits of each `odc` field after the magic.
const ODC_WIDTHS: [usize; 10] = [6, 6, 6, 6, 6, 6, 6, 11, 6, 11];

/// An `odc` header: magic and ten octal fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OdcHeader(pub [u8; ODC_HEADER_LEN]);

impl OdcHeader {
    /// Builds a header, or `None` when a value does not fit its field.
    pub fn new(fields: &OdcFields) -> Option<Self> {
        let mut data = [0u8; ODC_HEADER_LEN];
        data[..6].copy_from_slice(ODC_MAGIC);
        let values = [
            u64::from(fields.dev),
            u64::from(fields.ino),
            u64::from(fields.mode),
            u64::from(fields.uid),
            u64::from(fields.gid),
            u64::from(fields.nlink),
            u64::from(fields.rdev),
            fields.mtime,
            u64::from(fields.namesize),
            fields.filesize,
        ];
        let mut start = 6;
        for (value, width) in values.iter().zip(ODC_WIDTHS) {
            write_octal(*value, &mut data[start..start + width])?;
            start += width;
        }
        Some(Self(data))
    }

    /// The fields, or `None` when the magic is not `070707` or a field is
    /// not octal.
    pub fn fields(&self) -> Option<OdcFields> {
        if &self.0[..6] != ODC_MAGIC {
            return None;
        }
        let mut values = [0u64; 10];
        let mut start = 6;
        for (value, width) in values.iter_mut().zip(ODC_WIDTHS) {
            *value = parse_octal(&self.0[start..start + width])?;
            start += width;
        }
        let small = |value: u64| u32::try_from(value).ok();
        Some(OdcFields {
            dev: small(values[0])?,
            ino: small(values[1])?,
            mode: small(values[2])?,
            uid: small(values[3])?,
            gid: small(values[4])?,
            nlink: small(values[5])?,
            rdev: small(values[6])?,
            mtime: values[7],
            namesize: small(values[8])?,
            filesize: values[9],
        })
    }

    /// The largest value an `odc` field of `digits` octal digits holds.
    pub const fn max(digits: u32) -> u64 {
        (1u64 << (3 * digits)) - 1
    }
}

/// The fields of an old binary header, 16-bit words in the archive's byte
/// order. `mtime` and `filesize` are two words each, the high word first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BinaryFields {
    /// Device holding the file.
    pub dev: u16,
    /// Inode number.
    pub ino: u16,
    /// File type and permission bits.
    pub mode: u16,
    /// Owner user id.
    pub uid: u16,
    /// Owner group id.
    pub gid: u16,
    /// Number of links.
    pub nlink: u16,
    /// Device number of a device node.
    pub rdev: u16,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: u32,
    /// Length of the name with its NUL.
    pub namesize: u16,
    /// Length of the data.
    pub filesize: u32,
}

/// An old binary header. Hadris reads it and does not write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BinaryHeader(pub [u8; BINARY_HEADER_LEN]);

impl BinaryHeader {
    /// Whether the first two bytes are the binary magic in either byte order.
    pub fn is_magic(bytes: [u8; 2]) -> bool {
        u16::from_le_bytes(bytes) == BINARY_MAGIC || u16::from_be_bytes(bytes) == BINARY_MAGIC
    }

    /// The fields, in the byte order the magic shows, or `None` when the
    /// magic is wrong.
    pub fn fields(&self) -> Option<BinaryFields> {
        let little = match [self.0[0], self.0[1]] {
            bytes if u16::from_le_bytes(bytes) == BINARY_MAGIC => true,
            bytes if u16::from_be_bytes(bytes) == BINARY_MAGIC => false,
            _ => return None,
        };
        let word = |index: usize| {
            let bytes = [self.0[index * 2], self.0[index * 2 + 1]];
            if little {
                u16::from_le_bytes(bytes)
            } else {
                u16::from_be_bytes(bytes)
            }
        };
        let long = |index: usize| (u32::from(word(index)) << 16) | u32::from(word(index + 1));
        Some(BinaryFields {
            dev: word(1),
            ino: word(2),
            mode: word(3),
            uid: word(4),
            gid: word(5),
            nlink: word(6),
            rdev: word(7),
            mtime: long(8),
            namesize: word(10),
            filesize: long(11),
        })
    }
}

fn parse_hex(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for &byte in bytes {
        let digit = (byte as char).to_digit(16)?;
        value = (value << 4) | digit;
    }
    Some(value)
}

fn write_hex(value: u32, out: &mut [u8]) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    for (index, byte) in out.iter_mut().enumerate() {
        let shift = (7 - index) * 4;
        *byte = DIGITS[((value >> shift) & 0xF) as usize];
    }
}

fn parse_octal(bytes: &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for &byte in bytes {
        let digit = (byte as char).to_digit(8)?;
        value = (value << 3) | u64::from(digit);
    }
    Some(value)
}

fn write_octal(mut value: u64, out: &mut [u8]) -> Option<()> {
    for byte in out.iter_mut().rev() {
        *byte = b'0' + (value & 7) as u8;
        value >>= 3;
    }
    (value == 0).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newc_fields_roundtrip() {
        let fields = NewcFields {
            ino: 0x1234_5678,
            mode: 0o100644,
            filesize: u32::MAX,
            namesize: 6,
            check: 0xABCD,
            ..NewcFields::default()
        };
        let header = NewcHeader::new(true, &fields);
        assert!(header.is_crc());
        assert_eq!(&header.0[6..14], b"12345678");
        assert_eq!(header.fields(), Some(fields));
        let mut lower = header;
        lower.0[6..14].copy_from_slice(b"0000abcd");
        assert_eq!(lower.fields().unwrap().ino, 0xABCD);
        let mut bad = header;
        bad.0[20] = b'g';
        assert_eq!(bad.fields(), None);
        bad.0[..6].copy_from_slice(b"070703");
        assert_eq!(bad.fields(), None);
    }

    #[test]
    fn odc_fields_roundtrip() {
        let fields = OdcFields {
            ino: 9,
            mode: 0o100600,
            nlink: 1,
            mtime: OdcHeader::max(11),
            namesize: 2,
            filesize: 5,
            ..OdcFields::default()
        };
        let header = OdcHeader::new(&fields).unwrap();
        assert_eq!(&header.0[..6], ODC_MAGIC);
        assert_eq!(header.fields(), Some(fields));
        let too_big = OdcFields {
            uid: 1 << 18,
            ..fields
        };
        assert!(OdcHeader::new(&too_big).is_none());
        let mut bad = header;
        bad.0[10] = b'8';
        assert_eq!(bad.fields(), None);
    }

    #[test]
    fn binary_headers_read_in_both_byte_orders() {
        let words: [u16; 13] = [BINARY_MAGIC, 1, 2, 0o100644, 3, 4, 1, 0, 1, 2, 6, 0, 5];
        let mut le = [0u8; BINARY_HEADER_LEN];
        let mut be = [0u8; BINARY_HEADER_LEN];
        for (index, word) in words.iter().enumerate() {
            le[index * 2..index * 2 + 2].copy_from_slice(&word.to_le_bytes());
            be[index * 2..index * 2 + 2].copy_from_slice(&word.to_be_bytes());
        }
        let expected = BinaryFields {
            dev: 1,
            ino: 2,
            mode: 0o100644,
            uid: 3,
            gid: 4,
            nlink: 1,
            rdev: 0,
            mtime: 0x0001_0002,
            namesize: 6,
            filesize: 5,
        };
        assert_eq!(BinaryHeader(le).fields(), Some(expected));
        assert_eq!(BinaryHeader(be).fields(), Some(expected));
        assert!(BinaryHeader::is_magic([le[0], le[1]]));
        assert!(BinaryHeader::is_magic([be[0], be[1]]));
        assert!(!BinaryHeader::is_magic(*b"07"));
    }
}

use core::fmt;
use crate::error::{CpioError, Result};
use super::super::{Read, Write};

/// Magic bytes for the newc format (`070701`).
pub const MAGIC_NEWC: &[u8; 6] = b"070701";
/// Magic bytes for the newc+CRC format (`070702`).
pub const MAGIC_NEWC_CRC: &[u8; 6] = b"070702";
/// The sentinel filename that marks the end of the archive.
pub const TRAILER_NAME: &[u8] = b"TRAILER!!!";
/// Size of the raw newc header in bytes.
pub const HEADER_SIZE: usize = 110;

/// Identifies which newc variant an entry uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpioMagic {
    /// Standard newc format (`070701`).
    Newc,
    /// Newc with CRC checksums (`070702`).
    NewcCrc,
}

impl fmt::Display for CpioMagic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpioMagic::Newc => write!(f, "newc (070701)"),
            CpioMagic::NewcCrc => write!(f, "newc+CRC (070702)"),
        }
    }
}

impl CpioMagic {
    /// Returns the 6-byte magic string for this variant.
    pub fn as_bytes(&self) -> &[u8; 6] {
        match self {
            CpioMagic::Newc => MAGIC_NEWC,
            CpioMagic::NewcCrc => MAGIC_NEWC_CRC,
        }
    }
}

/// The raw 110-byte ASCII newc header.
///
/// Layout (all fields are 8-char uppercase hex, except magic which is 6):
/// ```text
///  Offset  Size  Field
///  0       6     magic
///  6       8     ino
///  14      8     mode
///  22      8     uid
///  30      8     gid
///  38      8     nlink
///  46      8     mtime
///  54      8     filesize
///  62      8     devmajor
///  70      8     devminor
///  78      8     rdevmajor
///  86      8     rdevminor
///  94      8     namesize
///  102     8     check
/// ```
pub struct RawNewcHeader {
    data: [u8; HEADER_SIZE],
}

io_transform! {

impl RawNewcHeader {
    /// Read and parse a 110-byte header from the given reader.
    pub async fn parse<R: Read>(reader: &mut R) -> Result<Self> {
        let mut data = [0u8; HEADER_SIZE];
        reader.read_exact(&mut data).await?;
        Ok(Self { data })
    }

    /// Write this header to the given writer.
    pub async fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.data).await?;
        Ok(())
    }
}

} // io_transform!

impl RawNewcHeader {
    /// Parse the magic bytes, returning `None` if they are not recognized.
    pub fn magic(&self) -> Option<CpioMagic> {
        let magic = &self.data[0..6];
        if magic == MAGIC_NEWC {
            Some(CpioMagic::Newc)
        } else if magic == MAGIC_NEWC_CRC {
            Some(CpioMagic::NewcCrc)
        } else {
            None
        }
    }

    /// Returns the raw 6-byte magic field (for error reporting).
    pub fn magic_bytes(&self) -> &[u8] {
        &self.data[0..6]
    }

    /// Returns the inode number of the entry.
    pub fn ino(&self) -> Result<u32> {
        parse_hex_field(&self.data[6..14], "ino")
    }

    /// Returns the file mode (file type and permission bits).
    pub fn mode(&self) -> Result<u32> {
        parse_hex_field(&self.data[14..22], "mode")
    }

    /// Returns the owner user ID.
    pub fn uid(&self) -> Result<u32> {
        parse_hex_field(&self.data[22..30], "uid")
    }

    /// Returns the owner group ID.
    pub fn gid(&self) -> Result<u32> {
        parse_hex_field(&self.data[30..38], "gid")
    }

    /// Returns the number of hard links to this file.
    pub fn nlink(&self) -> Result<u32> {
        parse_hex_field(&self.data[38..46], "nlink")
    }

    /// Returns the modification time as seconds since the Unix epoch.
    pub fn mtime(&self) -> Result<u32> {
        parse_hex_field(&self.data[46..54], "mtime")
    }

    /// Returns the file data size in bytes.
    pub fn filesize(&self) -> Result<u32> {
        parse_hex_field(&self.data[54..62], "filesize")
    }

    /// Returns the major number of the device containing this file.
    pub fn devmajor(&self) -> Result<u32> {
        parse_hex_field(&self.data[62..70], "devmajor")
    }

    /// Returns the minor number of the device containing this file.
    pub fn devminor(&self) -> Result<u32> {
        parse_hex_field(&self.data[70..78], "devminor")
    }

    /// Returns the major number of the device (for device node entries).
    pub fn rdevmajor(&self) -> Result<u32> {
        parse_hex_field(&self.data[78..86], "rdevmajor")
    }

    /// Returns the minor number of the device (for device node entries).
    pub fn rdevminor(&self) -> Result<u32> {
        parse_hex_field(&self.data[86..94], "rdevminor")
    }

    /// Returns the length of the filename in bytes (including the NUL terminator).
    pub fn namesize(&self) -> Result<u32> {
        parse_hex_field(&self.data[94..102], "namesize")
    }

    /// Returns the CRC checksum (only meaningful for `070702` format entries).
    pub fn check(&self) -> Result<u32> {
        parse_hex_field(&self.data[102..110], "check")
    }

    /// Build a new raw header from individual field values.
    pub fn build(
        magic: CpioMagic,
        ino: u32,
        mode: u32,
        uid: u32,
        gid: u32,
        nlink: u32,
        mtime: u32,
        filesize: u32,
        devmajor: u32,
        devminor: u32,
        rdevmajor: u32,
        rdevminor: u32,
        namesize: u32,
        check: u32,
    ) -> Self {
        let mut data = [0u8; HEADER_SIZE];
        data[0..6].copy_from_slice(magic.as_bytes());
        write_hex_field(ino, &mut data[6..14]);
        write_hex_field(mode, &mut data[14..22]);
        write_hex_field(uid, &mut data[22..30]);
        write_hex_field(gid, &mut data[30..38]);
        write_hex_field(nlink, &mut data[38..46]);
        write_hex_field(mtime, &mut data[46..54]);
        write_hex_field(filesize, &mut data[54..62]);
        write_hex_field(devmajor, &mut data[62..70]);
        write_hex_field(devminor, &mut data[70..78]);
        write_hex_field(rdevmajor, &mut data[78..86]);
        write_hex_field(rdevminor, &mut data[86..94]);
        write_hex_field(namesize, &mut data[94..102]);
        write_hex_field(check, &mut data[102..110]);
        Self { data }
    }
}

fn parse_hex_field(bytes: &[u8], field: &'static str) -> Result<u32> {
    let mut value: u32 = 0;
    for &b in bytes {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'f' => (b - b'a' + 10) as u32,
            b'A'..=b'F' => (b - b'A' + 10) as u32,
            _ => return Err(CpioError::InvalidHexField { field }),
        };
        value = value.checked_shl(4).ok_or(CpioError::InvalidHexField { field })? | digit;
    }
    Ok(value)
}

fn write_hex_field(value: u32, out: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    debug_assert!(out.len() == 8);
    for i in 0..8 {
        let shift = (7 - i) * 4;
        out[i] = HEX[((value >> shift) & 0xF) as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_roundtrip() {
        let mut buf = [0u8; 8];
        write_hex_field(0x12345678, &mut buf);
        assert_eq!(&buf, b"12345678");
        assert_eq!(parse_hex_field(&buf, "test").unwrap(), 0x12345678);

        write_hex_field(0, &mut buf);
        assert_eq!(&buf, b"00000000");
        assert_eq!(parse_hex_field(&buf, "test").unwrap(), 0);

        write_hex_field(0xFFFFFFFF, &mut buf);
        assert_eq!(&buf, b"FFFFFFFF");
        assert_eq!(parse_hex_field(&buf, "test").unwrap(), 0xFFFFFFFF);
    }

    #[test]
    fn test_lowercase_hex_parse() {
        assert_eq!(parse_hex_field(b"0000000a", "test").unwrap(), 10);
        assert_eq!(parse_hex_field(b"000000ff", "test").unwrap(), 255);
    }

    #[test]
    fn test_invalid_hex() {
        assert!(parse_hex_field(b"0000000g", "test").is_err());
    }
}

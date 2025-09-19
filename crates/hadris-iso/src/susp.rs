//! System Use Sharing Protocol

use hadris_io::{self as io, Parsable, Read, Seek, ReadExt, Writable};

use crate::types::U32LsbMsb;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SystemUseHeader {
    pub sig: [u8; 2],
    pub length: u8,
    pub version: u8,
}

impl Writable for SystemUseHeader {
    fn write<R: io::Write>(&self, writer: &mut R) -> io::Result<()> {
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

pub trait SystemUseEntry: Writable {
    const SIG: &'static [u8; 2];

    fn header(&self) -> SystemUseHeader;
    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self>;
}

/// SPEC: SUSP 5.1 Description of the CE System Use Field
///
/// If this field is specified, then the System Use area is specified in another sector
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ContinuationArea {
    /// The Sector of the Continuation System Use area
    pub sector: U32LsbMsb,
    /// The offset within the sector for the start of the SU
    pub offset: U32LsbMsb,
    pub length: U32LsbMsb,
}

impl SystemUseEntry for ContinuationArea {
    const SIG: &'static [u8; 2] = b"CE";
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 28,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        assert_eq!(&header.sig, Self::SIG);
        data.read_struct()
    }
}

impl Writable for ContinuationArea {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PaddingField {
    pub length: u8,
}

impl SystemUseEntry for PaddingField {
    const SIG: &'static [u8; 2] = b"PD";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 4 + self.length,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        let len = header.length - 4;
        let mut buf = [0];
        for _ in 0..len {
            data.read_exact(&mut buf)?;
        }
        Ok(Self { length: len })
    }
}

impl Writable for PaddingField {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> io::Result<()> {
        self.header().write(writer)?;
        // TODO: Maybe use buffered writer here
        for _ in 0..self.length {
            writer.write_all(&[0x00])?;
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SuspIdentifier {
    pub check_bytes: [u8; 2],
    pub bytes_skipped: u8,
}

impl SuspIdentifier {
    pub fn new(bytes_skipped: u8) -> Self {
        Self {
            check_bytes: [0xBE, 0xEF],
            bytes_skipped,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.check_bytes == [0xBE, 0xEF]
    }
}

impl SystemUseEntry for SuspIdentifier {
    const SIG: &'static [u8; 2] = b"SP";
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 7,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        assert_eq!(&header.sig, Self::SIG);
        data.read_struct()
    }
}

impl Writable for SuspIdentifier {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> std::io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuspTerminator;

impl SystemUseEntry for SuspTerminator {
    const SIG: &'static [u8; 2] = b"ST";
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 4,
            version: 1,
        }
    }

    fn parse_data<R: Read>(_header: SystemUseHeader, _data: &mut R) -> hadris_io::Result<Self> {
        Ok(Self)
    }
}

impl Writable for SuspTerminator {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> std::io::Result<()> {
        self.header().write(writer)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ExtensionReference {
    pub version: u8,
    pub identifier_len: u8,
    pub descriptor_len: u8,
    pub source_len: u8,
    pub buf: [u8; 252],
}

impl SystemUseEntry for ExtensionReference {
    const SIG: &'static [u8; 2] = b"ER";
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 8 + self.identifier_len + self.descriptor_len + self.source_len,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> hadris_io::Result<Self> {
        assert_eq!(&header.sig, Self::SIG);
        let mut buf = [0u8; 252];
        let meta: [u8; 4] = data.read_struct()?;
        data.read_exact(&mut buf[0..header.length as usize - size_of::<SystemUseHeader>()])?;
        Ok(Self {
            identifier_len: meta[0],
            descriptor_len: meta[1],
            source_len: meta[2],
            version: meta[3],
            buf,
        })
    }
}

impl Writable for ExtensionReference {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> std::io::Result<()> {
        self.header().write(writer)?;
        let meta = [
            self.identifier_len,
            self.descriptor_len,
            self.source_len,
            self.version,
        ];
        writer.write_all(&meta)?;
        let total_len = self.identifier_len + self.descriptor_len + self.source_len;
        writer.write_all(&self.buf[0..total_len as usize])?;
        Ok(())
    }
}

pub enum SystemUseField {
    Unknown(SystemUseHeader),
}

pub struct SystemUseIter {
}

impl SystemUseIter {

}

impl Iterator for SystemUseField {
    type Item = SystemUseField;

    fn next(&mut self) -> Option<Self::Item> {
        todo!() 
    }
}

#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<SystemUseHeader>(), 4);
    static_assertions::const_assert_eq!(size_of::<ContinuationArea>(), 24);
    static_assertions::const_assert_eq!(size_of::<SuspIdentifier>(), 3);
}

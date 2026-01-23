//! System Use Sharing Protocol

use hadris_io::{self as io, Read, ReadExt, Writable};

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

/// A parsed system use field
#[derive(Debug, Clone)]
pub enum SystemUseField {
    /// SP - SUSP Identifier entry
    SuspIdentifier(SuspIdentifier),
    /// ST - Terminator entry
    Terminator,
    /// CE - Continuation Area entry
    ContinuationArea(ContinuationArea),
    /// PD - Padding entry
    Padding(PaddingField),
    /// ER - Extension Reference entry
    ExtensionReference(ExtensionReference),
    /// Unknown entry (not recognized)
    Unknown(SystemUseHeader, [u8; 252]),
}

/// Iterator over system use entries in a byte slice
pub struct SystemUseIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> SystemUseIter<'a> {
    /// Create a new iterator over system use entries
    ///
    /// # Arguments
    /// * `data` - The system use area bytes
    /// * `bytes_to_skip` - Number of bytes to skip at the start (from SP entry)
    pub fn new(data: &'a [u8], bytes_to_skip: usize) -> Self {
        Self {
            data,
            offset: bytes_to_skip,
        }
    }
}

impl Iterator for SystemUseIter<'_> {
    type Item = SystemUseField;

    fn next(&mut self) -> Option<Self::Item> {
        // Need at least 4 bytes for the header
        if self.offset + 4 > self.data.len() {
            return None;
        }

        let header_bytes = &self.data[self.offset..self.offset + 4];
        let header: SystemUseHeader = *bytemuck::from_bytes(header_bytes);

        // Zero length means end of entries
        if header.length == 0 {
            return None;
        }

        // Validate we have enough data
        let entry_end = self.offset + header.length as usize;
        if entry_end > self.data.len() {
            return None;
        }

        let entry_data = &self.data[self.offset + 4..entry_end];
        self.offset = entry_end;

        let field = match &header.sig {
            b"SP" if entry_data.len() >= 3 => {
                let sp: SuspIdentifier = *bytemuck::from_bytes(&entry_data[0..3]);
                SystemUseField::SuspIdentifier(sp)
            }
            b"ST" => SystemUseField::Terminator,
            b"CE" if entry_data.len() >= 24 => {
                let ce: ContinuationArea = *bytemuck::from_bytes(&entry_data[0..24]);
                SystemUseField::ContinuationArea(ce)
            }
            b"PD" => {
                let len = header.length.saturating_sub(4);
                SystemUseField::Padding(PaddingField { length: len })
            }
            b"ER" if entry_data.len() >= 4 => {
                let mut buf = [0u8; 252];
                let copy_len = entry_data.len().min(252);
                buf[..copy_len].copy_from_slice(&entry_data[..copy_len]);
                SystemUseField::ExtensionReference(ExtensionReference {
                    identifier_len: entry_data[0],
                    descriptor_len: entry_data[1],
                    source_len: entry_data[2],
                    version: entry_data[3],
                    buf,
                })
            }
            _ => {
                let mut buf = [0u8; 252];
                let copy_len = entry_data.len().min(252);
                buf[..copy_len].copy_from_slice(&entry_data[..copy_len]);
                SystemUseField::Unknown(header, buf)
            }
        };

        Some(field)
    }
}

/// Builder for constructing system use areas
#[cfg(feature = "alloc")]
pub struct SystemUseBuilder {
    entries: alloc::vec::Vec<alloc::vec::Vec<u8>>,
}

#[cfg(feature = "alloc")]
impl SystemUseBuilder {
    /// Create a new empty builder
    pub fn new() -> Self {
        Self {
            entries: alloc::vec::Vec::new(),
        }
    }

    /// Add the SP (SUSP Identifier) entry
    ///
    /// This should be the first entry in the root directory's system use area
    pub fn add_sp(&mut self, bytes_skipped: u8) -> &mut Self {
        let sp = SuspIdentifier::new(bytes_skipped);
        let mut buf = alloc::vec![0u8; 7];
        buf[0..2].copy_from_slice(b"SP");
        buf[2] = 7;  // length
        buf[3] = 1;  // version
        buf[4..7].copy_from_slice(bytemuck::bytes_of(&sp));
        self.entries.push(buf);
        self
    }

    /// Add the ST (Terminator) entry
    pub fn add_st(&mut self) -> &mut Self {
        let buf = alloc::vec![b'S', b'T', 4, 1];
        self.entries.push(buf);
        self
    }

    /// Add a CE (Continuation Area) entry
    pub fn add_ce(&mut self, ce: ContinuationArea) -> &mut Self {
        let mut buf = alloc::vec![0u8; 28];
        buf[0..2].copy_from_slice(b"CE");
        buf[2] = 28;
        buf[3] = 1;
        buf[4..28].copy_from_slice(bytemuck::bytes_of(&ce));
        self.entries.push(buf);
        self
    }

    /// Add padding to align the system use area
    pub fn add_padding(&mut self, length: u8) -> &mut Self {
        let mut buf = alloc::vec![0u8; 4 + length as usize];
        buf[0..2].copy_from_slice(b"PD");
        buf[2] = 4 + length;
        buf[3] = 1;
        self.entries.push(buf);
        self
    }

    /// Add an ER (Extension Reference) entry
    pub fn add_er(&mut self, identifier: &str, descriptor: &str, source: &str, version: u8) -> &mut Self {
        let id_bytes = identifier.as_bytes();
        let desc_bytes = descriptor.as_bytes();
        let src_bytes = source.as_bytes();

        let len = 8 + id_bytes.len() + desc_bytes.len() + src_bytes.len();
        let mut buf = alloc::vec![0u8; len];
        buf[0..2].copy_from_slice(b"ER");
        buf[2] = len as u8;
        buf[3] = 1;
        buf[4] = id_bytes.len() as u8;
        buf[5] = desc_bytes.len() as u8;
        buf[6] = src_bytes.len() as u8;
        buf[7] = version;

        let mut offset = 8;
        buf[offset..offset + id_bytes.len()].copy_from_slice(id_bytes);
        offset += id_bytes.len();
        buf[offset..offset + desc_bytes.len()].copy_from_slice(desc_bytes);
        offset += desc_bytes.len();
        buf[offset..offset + src_bytes.len()].copy_from_slice(src_bytes);

        self.entries.push(buf);
        self
    }

    /// Add raw bytes for a custom entry
    pub fn add_raw(&mut self, data: alloc::vec::Vec<u8>) -> &mut Self {
        self.entries.push(data);
        self
    }

    /// Calculate the total size of the system use area
    pub fn size(&self) -> usize {
        self.entries.iter().map(|e| e.len()).sum()
    }

    /// Build the system use area as a byte vector
    pub fn build(&self) -> alloc::vec::Vec<u8> {
        let total_size = self.size();
        let mut result = alloc::vec::Vec::with_capacity(total_size);
        for entry in &self.entries {
            result.extend_from_slice(entry);
        }
        result
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(feature = "alloc")]
impl Default for SystemUseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    static_assertions::const_assert_eq!(size_of::<SystemUseHeader>(), 4);
    static_assertions::const_assert_eq!(size_of::<ContinuationArea>(), 24);
    static_assertions::const_assert_eq!(size_of::<SuspIdentifier>(), 3);

    #[test]
    fn test_susp_identifier_valid() {
        let sp = SuspIdentifier::new(0);
        assert!(sp.is_valid());
        assert_eq!(sp.check_bytes, [0xBE, 0xEF]);
        assert_eq!(sp.bytes_skipped, 0);
    }

    #[test]
    fn test_susp_identifier_invalid() {
        let sp = SuspIdentifier {
            check_bytes: [0x00, 0x00],
            bytes_skipped: 0,
        };
        assert!(!sp.is_valid());
    }

    #[test]
    fn test_system_use_iter_empty() {
        let data: &[u8] = &[];
        let mut iter = SystemUseIter::new(data, 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_sp_entry() {
        // SP entry: sig='SP', length=7, version=1, check_bytes=[0xBE, 0xEF], bytes_skipped=0
        let data: &[u8] = &[
            b'S', b'P', 7, 1,  // header
            0xBE, 0xEF, 0,     // data
        ];
        let mut iter = SystemUseIter::new(data, 0);
        match iter.next() {
            Some(SystemUseField::SuspIdentifier(sp)) => {
                assert!(sp.is_valid());
                assert_eq!(sp.bytes_skipped, 0);
            }
            other => panic!("expected SP entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_st_entry() {
        // ST entry: sig='ST', length=4, version=1
        let data: &[u8] = &[b'S', b'T', 4, 1];
        let mut iter = SystemUseIter::new(data, 0);
        match iter.next() {
            Some(SystemUseField::Terminator) => {}
            other => panic!("expected ST entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_ce_entry() {
        // CE entry: sig='CE', length=28, version=1, then 24 bytes of data
        let mut data = vec![b'C', b'E', 28, 1];
        // sector (8 bytes), offset (8 bytes), length (8 bytes)
        data.extend_from_slice(&[10, 0, 0, 0, 10, 0, 0, 0]); // sector = 10
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);   // offset = 0
        data.extend_from_slice(&[100, 0, 0, 0, 100, 0, 0, 0]); // length = 100

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::ContinuationArea(ce)) => {
                assert_eq!(ce.sector.read(), 10);
                assert_eq!(ce.offset.read(), 0);
                assert_eq!(ce.length.read(), 100);
            }
            other => panic!("expected CE entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_pd_entry() {
        // PD entry: sig='PD', length=8, version=1, then 4 bytes of padding
        let data: &[u8] = &[b'P', b'D', 8, 1, 0, 0, 0, 0];
        let mut iter = SystemUseIter::new(data, 0);
        match iter.next() {
            Some(SystemUseField::Padding(pd)) => {
                assert_eq!(pd.length, 4);
            }
            other => panic!("expected PD entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_unknown_entry() {
        // Unknown entry: sig='XX', length=6, version=1, data=[0x42, 0x43]
        let data: &[u8] = &[b'X', b'X', 6, 1, 0x42, 0x43];
        let mut iter = SystemUseIter::new(data, 0);
        match iter.next() {
            Some(SystemUseField::Unknown(header, buf)) => {
                assert_eq!(&header.sig, b"XX");
                assert_eq!(header.length, 6);
                assert_eq!(&buf[0..2], &[0x42, 0x43]);
            }
            other => panic!("expected Unknown entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_multiple_entries() {
        // SP followed by ST
        let data: &[u8] = &[
            b'S', b'P', 7, 1, 0xBE, 0xEF, 0,  // SP
            b'S', b'T', 4, 1,                  // ST
        ];
        let mut iter = SystemUseIter::new(data, 0);
        assert!(matches!(iter.next(), Some(SystemUseField::SuspIdentifier(_))));
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_with_skip() {
        // First 3 bytes should be skipped
        let data: &[u8] = &[
            0xFF, 0xFF, 0xFF,                 // skip these
            b'S', b'T', 4, 1,                 // ST
        ];
        let mut iter = SystemUseIter::new(data, 3);
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_zero_length_terminates() {
        let data: &[u8] = &[0, 0, 0, 0];  // zero-length header
        let mut iter = SystemUseIter::new(data, 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_builder_empty() {
        let builder = SystemUseBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.size(), 0);
        assert_eq!(builder.build(), Vec::<u8>::new());
    }

    #[test]
    fn test_system_use_builder_sp() {
        let mut builder = SystemUseBuilder::new();
        builder.add_sp(0);
        assert!(!builder.is_empty());
        assert_eq!(builder.size(), 7);

        let data = builder.build();
        assert_eq!(&data[0..2], b"SP");
        assert_eq!(data[2], 7);  // length
        assert_eq!(data[3], 1);  // version
        assert_eq!(&data[4..6], &[0xBE, 0xEF]);  // check bytes
        assert_eq!(data[6], 0);  // bytes_skipped
    }

    #[test]
    fn test_system_use_builder_st() {
        let mut builder = SystemUseBuilder::new();
        builder.add_st();
        assert_eq!(builder.size(), 4);

        let data = builder.build();
        assert_eq!(&data, &[b'S', b'T', 4, 1]);
    }

    #[test]
    fn test_system_use_builder_padding() {
        let mut builder = SystemUseBuilder::new();
        builder.add_padding(4);
        assert_eq!(builder.size(), 8);

        let data = builder.build();
        assert_eq!(&data[0..2], b"PD");
        assert_eq!(data[2], 8);  // length = 4 + padding
        assert_eq!(data[3], 1);  // version
    }

    #[test]
    fn test_system_use_builder_er() {
        let mut builder = SystemUseBuilder::new();
        builder.add_er("RRIP", "Rock Ridge", "TEST", 1);

        let data = builder.build();
        assert_eq!(&data[0..2], b"ER");
        let len = data[2] as usize;
        assert_eq!(len, 8 + 4 + 10 + 4);  // header(8) + id(4) + desc(10) + src(4)
        assert_eq!(data[3], 1);  // version
        assert_eq!(data[4], 4);  // id_len
        assert_eq!(data[5], 10); // desc_len
        assert_eq!(data[6], 4);  // src_len
        assert_eq!(data[7], 1);  // ext_version

        let id = &data[8..12];
        assert_eq!(id, b"RRIP");
    }

    #[test]
    fn test_system_use_builder_roundtrip() {
        let mut builder = SystemUseBuilder::new();
        builder.add_sp(0).add_st();

        let data = builder.build();
        let mut iter = SystemUseIter::new(&data, 0);

        assert!(matches!(iter.next(), Some(SystemUseField::SuspIdentifier(_))));
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }
}

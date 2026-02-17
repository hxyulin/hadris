//! System Use Sharing Protocol

use hadris_io::{self as io, Read, ReadExt};
#[cfg(feature = "std")]
use hadris_io::Writable;

use crate::types::U32LsbMsb;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SystemUseHeader {
    pub sig: [u8; 2],
    pub length: u8,
    pub version: u8,
}

#[cfg(feature = "std")]
impl Writable for SystemUseHeader {
    fn write<R: io::Write>(&self, writer: &mut R) -> io::Result<()> {
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
impl Writable for PaddingField {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> io::Result<()> {
        self.header().write(writer)?;
        let zeros = alloc::vec![0u8; self.length as usize];
        writer.write_all(&zeros)?;
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
impl Writable for SuspIdentifier {
    fn write<R: std::io::Write>(&self, writer: &mut R) -> std::io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuspTerminator;

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

    // --- RRIP variants ---
    /// PX - POSIX file attributes
    PosixAttributes(crate::rrip::PxEntry),
    /// PN - POSIX device numbers
    DeviceNumber(crate::rrip::PnEntry),
    /// NM - Alternate name (real filename)
    AlternateName(crate::rrip::NmEntry),
    /// SL - Symbolic link
    SymbolicLink(crate::rrip::SlEntry),
    /// TF - Timestamps
    Timestamps(crate::rrip::TfEntry),
    /// CL - Child link (relocated directory)
    ChildLink(crate::rrip::ClEntry),
    /// PL - Parent link (relocated directory)
    ParentLink(crate::rrip::PlEntry),
    /// RE - Relocated directory marker
    Relocated,

    /// ES - Extension Selector
    ExtensionSelector { extension_sequence: u8 },

    /// Unknown entry (not recognized)
    Unknown(SystemUseHeader, [u8; 252]),
}

impl SystemUseField {
    /// Returns the 2-byte signature for this field.
    pub fn signature(&self) -> [u8; 2] {
        match self {
            Self::SuspIdentifier(_) => *b"SP",
            Self::Terminator => *b"ST",
            Self::ContinuationArea(_) => *b"CE",
            Self::Padding(_) => *b"PD",
            Self::ExtensionReference(_) => *b"ER",
            Self::PosixAttributes(_) => *b"PX",
            Self::DeviceNumber(_) => *b"PN",
            Self::AlternateName(_) => *b"NM",
            Self::SymbolicLink(_) => *b"SL",
            Self::Timestamps(_) => *b"TF",
            Self::ChildLink(_) => *b"CL",
            Self::ParentLink(_) => *b"PL",
            Self::Relocated => *b"RE",
            Self::ExtensionSelector { .. } => *b"ES",
            Self::Unknown(header, _) => header.sig,
        }
    }

    /// Returns the PX entry if this is a `PosixAttributes` variant.
    pub fn as_posix_attributes(&self) -> Option<&crate::rrip::PxEntry> {
        match self {
            Self::PosixAttributes(px) => Some(px),
            _ => None,
        }
    }

    /// Returns the NM entry if this is an `AlternateName` variant.
    pub fn as_alternate_name(&self) -> Option<&crate::rrip::NmEntry> {
        match self {
            Self::AlternateName(nm) => Some(nm),
            _ => None,
        }
    }

    /// Returns the TF entry if this is a `Timestamps` variant.
    pub fn as_timestamps(&self) -> Option<&crate::rrip::TfEntry> {
        match self {
            Self::Timestamps(tf) => Some(tf),
            _ => None,
        }
    }

    /// Returns the SL entry if this is a `SymbolicLink` variant.
    pub fn as_symbolic_link(&self) -> Option<&crate::rrip::SlEntry> {
        match self {
            Self::SymbolicLink(sl) => Some(sl),
            _ => None,
        }
    }
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
            // --- RRIP entries ---
            b"PX" if entry_data.len() >= 32 => {
                let len = entry_data.len().min(40);
                let px = crate::rrip::PxEntry {
                    file_mode: *bytemuck::from_bytes(&entry_data[0..8]),
                    file_links: *bytemuck::from_bytes(&entry_data[8..16]),
                    file_uid: *bytemuck::from_bytes(&entry_data[16..24]),
                    file_gid: *bytemuck::from_bytes(&entry_data[24..32]),
                    file_serial: if len >= 40 {
                        *bytemuck::from_bytes(&entry_data[32..40])
                    } else {
                        crate::types::U32LsbMsb::new(0)
                    },
                };
                SystemUseField::PosixAttributes(px)
            }
            b"PN" if entry_data.len() >= 16 => {
                let pn = crate::rrip::PnEntry {
                    dev_high: *bytemuck::from_bytes(&entry_data[0..8]),
                    dev_low: *bytemuck::from_bytes(&entry_data[8..16]),
                };
                SystemUseField::DeviceNumber(pn)
            }
            b"NM" if !entry_data.is_empty() => {
                let flags = crate::rrip::NmFlags::from_bits_truncate(entry_data[0]);
                let name = entry_data[1..].to_vec();
                SystemUseField::AlternateName(crate::rrip::NmEntry { flags, name })
            }
            b"SL" if !entry_data.is_empty() => {
                let flags = entry_data[0];
                let mut components = alloc::vec::Vec::new();
                let mut pos = 1;
                while pos + 2 <= entry_data.len() {
                    let comp_flags =
                        crate::rrip::SlComponentFlags::from_bits_truncate(entry_data[pos]);
                    let comp_len = entry_data[pos + 1] as usize;
                    pos += 2;
                    let end = (pos + comp_len).min(entry_data.len());
                    let content = entry_data[pos..end].to_vec();
                    pos = end;
                    components.push(crate::rrip::SlComponent {
                        flags: comp_flags,
                        content,
                    });
                }
                SystemUseField::SymbolicLink(crate::rrip::SlEntry { flags, components })
            }
            b"TF" if !entry_data.is_empty() => {
                let flags = crate::rrip::TfFlags::from_bits_truncate(entry_data[0]);
                let timestamps = entry_data[1..].to_vec();
                SystemUseField::Timestamps(crate::rrip::TfEntry { flags, timestamps })
            }
            b"CL" if entry_data.len() >= 8 => {
                let cl = crate::rrip::ClEntry {
                    child_directory_location: *bytemuck::from_bytes(&entry_data[0..8]),
                };
                SystemUseField::ChildLink(cl)
            }
            b"PL" if entry_data.len() >= 8 => {
                let pl = crate::rrip::PlEntry {
                    parent_directory_location: *bytemuck::from_bytes(&entry_data[0..8]),
                };
                SystemUseField::ParentLink(pl)
            }
            b"RE" => SystemUseField::Relocated,
            b"ES" if !entry_data.is_empty() => {
                SystemUseField::ExtensionSelector {
                    extension_sequence: entry_data[0],
                }
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
        buf[2] = 7; // length
        buf[3] = 1; // version
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
    pub fn add_er(
        &mut self,
        identifier: &str,
        descriptor: &str,
        source: &str,
        version: u8,
    ) -> &mut Self {
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

    /// Split entries across inline and overflow areas.
    ///
    /// Walks entries in order, keeping as many complete entries inline as
    /// possible. Remaining entries go to a continuation area, and a CE
    /// pointer is appended to the inline portion.
    pub fn build_split(&self, max_inline: usize) -> SplitSu {
        let total = self.size();
        if total <= max_inline {
            return SplitSu {
                inline: self.build(),
                overflow: alloc::vec::Vec::new(),
                ce_offset: None,
            };
        }

        const CE_SIZE: usize = 28;
        let budget = max_inline.saturating_sub(CE_SIZE);

        // Find split point: largest prefix of entries fitting in budget
        let mut accumulated = 0;
        let mut split_at = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            if accumulated + entry.len() > budget {
                split_at = i;
                break;
            }
            accumulated += entry.len();
            split_at = i + 1;
        }

        // Build inline = entries[..split_at] + CE placeholder
        let mut inline = alloc::vec::Vec::with_capacity(accumulated + CE_SIZE);
        for entry in &self.entries[..split_at] {
            inline.extend_from_slice(entry);
        }
        let ce_offset = inline.len();
        // CE placeholder: header (sig+len+ver) + 24 zeroed bytes for sector/offset/length
        let mut ce_buf = alloc::vec![0u8; CE_SIZE];
        ce_buf[0..2].copy_from_slice(b"CE");
        ce_buf[2] = CE_SIZE as u8;
        ce_buf[3] = 1;
        inline.extend_from_slice(&ce_buf);

        // Build overflow = entries[split_at..]
        let overflow_size: usize = self.entries[split_at..].iter().map(|e| e.len()).sum();
        let mut overflow = alloc::vec::Vec::with_capacity(overflow_size);
        for entry in &self.entries[split_at..] {
            overflow.extend_from_slice(entry);
        }

        SplitSu {
            inline,
            overflow,
            ce_offset: Some(ce_offset),
        }
    }
}

/// Result of splitting a system use area into inline and overflow portions.
#[cfg(feature = "alloc")]
pub struct SplitSu {
    /// Bytes that fit inline in the directory record's SU area.
    pub inline: alloc::vec::Vec<u8>,
    /// Bytes that go in the continuation area (may be empty).
    pub overflow: alloc::vec::Vec<u8>,
    /// Byte offset of the CE entry within `inline`, for patching.
    ce_offset: Option<usize>,
}

#[cfg(feature = "alloc")]
impl SplitSu {
    /// Create an empty SplitSu with no inline or overflow data.
    pub fn empty() -> Self {
        Self {
            inline: alloc::vec::Vec::new(),
            overflow: alloc::vec::Vec::new(),
            ce_offset: None,
        }
    }

    /// Returns true if there is overflow data requiring a continuation area.
    pub fn has_overflow(&self) -> bool {
        !self.overflow.is_empty()
    }

    /// Patch the CE entry with the actual continuation area location.
    pub fn patch_ce(&mut self, sector: u32, byte_offset: u32) {
        if let Some(off) = self.ce_offset {
            let data_start = off + 4; // skip header (sig+len+ver)
            let length = self.overflow.len() as u32;
            // Write sector as U32LsbMsb (4 LE + 4 BE = 8 bytes)
            self.inline[data_start..data_start + 4].copy_from_slice(&sector.to_le_bytes());
            self.inline[data_start + 4..data_start + 8].copy_from_slice(&sector.to_be_bytes());
            // Write offset as U32LsbMsb
            self.inline[data_start + 8..data_start + 12]
                .copy_from_slice(&byte_offset.to_le_bytes());
            self.inline[data_start + 12..data_start + 16]
                .copy_from_slice(&byte_offset.to_be_bytes());
            // Write length as U32LsbMsb
            self.inline[data_start + 16..data_start + 20].copy_from_slice(&length.to_le_bytes());
            self.inline[data_start + 20..data_start + 24].copy_from_slice(&length.to_be_bytes());
        }
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
            b'S', b'P', 7, 1, // header
            0xBE, 0xEF, 0, // data
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
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // offset = 0
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
            b'S', b'P', 7, 1, 0xBE, 0xEF, 0, // SP
            b'S', b'T', 4, 1, // ST
        ];
        let mut iter = SystemUseIter::new(data, 0);
        assert!(matches!(
            iter.next(),
            Some(SystemUseField::SuspIdentifier(_))
        ));
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_with_skip() {
        // First 3 bytes should be skipped
        let data: &[u8] = &[
            0xFF, 0xFF, 0xFF, // skip these
            b'S', b'T', 4, 1, // ST
        ];
        let mut iter = SystemUseIter::new(data, 3);
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_system_use_iter_zero_length_terminates() {
        let data: &[u8] = &[0, 0, 0, 0]; // zero-length header
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
        assert_eq!(data[2], 7); // length
        assert_eq!(data[3], 1); // version
        assert_eq!(&data[4..6], &[0xBE, 0xEF]); // check bytes
        assert_eq!(data[6], 0); // bytes_skipped
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
        assert_eq!(data[2], 8); // length = 4 + padding
        assert_eq!(data[3], 1); // version
    }

    #[test]
    fn test_system_use_builder_er() {
        let mut builder = SystemUseBuilder::new();
        builder.add_er("RRIP", "Rock Ridge", "TEST", 1);

        let data = builder.build();
        assert_eq!(&data[0..2], b"ER");
        let len = data[2] as usize;
        assert_eq!(len, 8 + 4 + 10 + 4); // header(8) + id(4) + desc(10) + src(4)
        assert_eq!(data[3], 1); // version
        assert_eq!(data[4], 4); // id_len
        assert_eq!(data[5], 10); // desc_len
        assert_eq!(data[6], 4); // src_len
        assert_eq!(data[7], 1); // ext_version

        let id = &data[8..12];
        assert_eq!(id, b"RRIP");
    }

    #[test]
    fn test_system_use_builder_roundtrip() {
        let mut builder = SystemUseBuilder::new();
        builder.add_sp(0).add_st();

        let data = builder.build();
        let mut iter = SystemUseIter::new(&data, 0);

        assert!(matches!(
            iter.next(),
            Some(SystemUseField::SuspIdentifier(_))
        ));
        assert!(matches!(iter.next(), Some(SystemUseField::Terminator)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_build_split_no_overflow() {
        // SP(7) + ST(4) = 11 bytes, fits in 100 bytes inline
        let mut builder = SystemUseBuilder::new();
        builder.add_sp(0).add_st();
        let split = builder.build_split(100);
        assert!(!split.has_overflow());
        assert!(split.overflow.is_empty());
        assert_eq!(split.inline, builder.build());
    }

    #[test]
    fn test_build_split_with_overflow() {
        // SP(7) + a large ER entry. SP should stay inline, ER should overflow.
        let mut builder = SystemUseBuilder::new();
        builder.add_sp(0);
        builder.add_er(
            "RRIP_1991A",
            "THE ROCK RIDGE INTERCHANGE PROTOCOL PROVIDES SUPPORT FOR POSIX FILE SYSTEM SEMANTICS",
            "PLEASE CONTACT DISC PUBLISHER FOR SPECIFICATION SOURCE. SEE PUBLISHER IDENTIFIER IN PRIMARY VOLUME DESCRIPTOR FOR CONTACT INFORMATION.",
            1,
        );
        let total = builder.size();
        assert!(total > 100); // ER is ~236 bytes

        // Give enough budget for SP(7) + CE(28) = 35 bytes inline
        let mut split = builder.build_split(40);
        assert!(split.has_overflow());
        // Inline should be: SP(7) + CE(28) = 35 bytes
        assert_eq!(split.inline.len(), 7 + 28);
        // Overflow should contain the ER entry
        assert_eq!(&split.overflow[0..2], b"ER");
        // Inline starts with SP
        assert_eq!(&split.inline[0..2], b"SP");
        // CE is at offset 7
        assert_eq!(&split.inline[7..9], b"CE");
        assert_eq!(split.inline[9], 28); // CE length

        // Patch CE and verify the fields are set correctly
        split.patch_ce(42, 100);
        let ce_data = &split.inline[7..35];
        // Header is 4 bytes (sig+len+ver), then sector(8), offset(8), length(8)
        let sector_le = u32::from_le_bytes(ce_data[4..8].try_into().unwrap());
        let sector_be = u32::from_be_bytes(ce_data[8..12].try_into().unwrap());
        assert_eq!(sector_le, 42);
        assert_eq!(sector_be, 42);
        let offset_le = u32::from_le_bytes(ce_data[12..16].try_into().unwrap());
        let offset_be = u32::from_be_bytes(ce_data[16..20].try_into().unwrap());
        assert_eq!(offset_le, 100);
        assert_eq!(offset_be, 100);
        let length_le = u32::from_le_bytes(ce_data[20..24].try_into().unwrap());
        let length_be = u32::from_be_bytes(ce_data[24..28].try_into().unwrap());
        assert_eq!(length_le, split.overflow.len() as u32);
        assert_eq!(length_be, split.overflow.len() as u32);
    }

    // ── RRIP parsing tests ──

    /// Helper: build a U32LsbMsb as 8 raw bytes (4 LE + 4 BE).
    fn u32_lsb_msb_bytes(v: u32) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&v.to_le_bytes());
        buf[4..8].copy_from_slice(&v.to_be_bytes());
        buf
    }

    #[test]
    fn test_iter_px_entry() {
        // PX entry: 44 bytes total (header 4 + 40 data bytes)
        let mut data = vec![b'P', b'X', 44, 1];
        data.extend_from_slice(&u32_lsb_msb_bytes(0o100644)); // file_mode
        data.extend_from_slice(&u32_lsb_msb_bytes(1)); // file_links
        data.extend_from_slice(&u32_lsb_msb_bytes(1000)); // file_uid
        data.extend_from_slice(&u32_lsb_msb_bytes(2000)); // file_gid
        data.extend_from_slice(&u32_lsb_msb_bytes(42)); // file_serial

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::PosixAttributes(px)) => {
                assert_eq!(px.file_mode.read(), 0o100644);
                assert_eq!(px.file_links.read(), 1);
                assert_eq!(px.file_uid.read(), 1000);
                assert_eq!(px.file_gid.read(), 2000);
                assert_eq!(px.file_serial.read(), 42);
            }
            other => panic!("expected PX entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_nm_entry() {
        // NM entry: header(4) + flags(1) + name bytes
        let name = b"hello.txt";
        let total_len = 4 + 1 + name.len();
        let mut data = vec![b'N', b'M', total_len as u8, 1];
        data.push(0x00); // flags: no special flags
        data.extend_from_slice(name);

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::AlternateName(nm)) => {
                assert!(nm.flags.is_empty());
                assert_eq!(nm.name, b"hello.txt");
            }
            other => panic!("expected NM entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_tf_entry() {
        // TF with MODIFY(0x02) + ACCESS(0x04) flags = 0x06, short form (7 bytes each)
        let total_len = 4 + 1 + 14; // header + flags + 2*7 timestamps
        let mut data = vec![b'T', b'F', total_len as u8, 1];
        data.push(0x06); // MODIFY | ACCESS
        // Modify timestamp: 2026-01-15 10:30:00 UTC
        data.extend_from_slice(&[126, 1, 15, 10, 30, 0, 0]);
        // Access timestamp: 2026-01-15 12:00:00 UTC
        data.extend_from_slice(&[126, 1, 15, 12, 0, 0, 0]);

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::Timestamps(tf)) => {
                use crate::rrip::TfFlags;
                assert!(tf.flags.contains(TfFlags::MODIFY));
                assert!(tf.flags.contains(TfFlags::ACCESS));
                assert!(!tf.flags.contains(TfFlags::LONG_FORM));
                assert_eq!(tf.timestamps.len(), 14);
            }
            other => panic!("expected TF entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_sl_entry() {
        // SL for "/usr/bin": root component + "usr" + "bin"
        let mut data = vec![b'S', b'L', 0, 1]; // length patched below
        data.push(0x00); // SL flags
        // Root component: flags=ROOT(0x08), len=0
        data.push(0x08);
        data.push(0x00);
        // "usr" component: flags=0, len=3
        data.push(0x00);
        data.push(0x03);
        data.extend_from_slice(b"usr");
        // "bin" component: flags=0, len=3
        data.push(0x00);
        data.push(0x03);
        data.extend_from_slice(b"bin");
        // Patch total length
        data[2] = data.len() as u8;

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::SymbolicLink(sl)) => {
                assert_eq!(sl.flags, 0);
                assert_eq!(sl.components.len(), 3);
                assert!(
                    sl.components[0]
                        .flags
                        .contains(crate::rrip::SlComponentFlags::ROOT)
                );
                assert!(sl.components[0].content.is_empty());
                assert_eq!(sl.components[1].content, b"usr");
                assert_eq!(sl.components[2].content, b"bin");
            }
            other => panic!("expected SL entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_cl_pl_re() {
        let mut data = Vec::new();
        // CL entry: header(4) + location(8) = 12
        data.extend_from_slice(&[b'C', b'L', 12, 1]);
        data.extend_from_slice(&u32_lsb_msb_bytes(100));
        // PL entry: header(4) + location(8) = 12
        data.extend_from_slice(&[b'P', b'L', 12, 1]);
        data.extend_from_slice(&u32_lsb_msb_bytes(50));
        // RE entry: header(4) = 4
        data.extend_from_slice(&[b'R', b'E', 4, 1]);

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::ChildLink(cl)) => {
                assert_eq!(cl.child_directory_location.read(), 100);
            }
            other => panic!("expected CL entry, got {:?}", other),
        }
        match iter.next() {
            Some(SystemUseField::ParentLink(pl)) => {
                assert_eq!(pl.parent_directory_location.read(), 50);
            }
            other => panic!("expected PL entry, got {:?}", other),
        }
        match iter.next() {
            Some(SystemUseField::Relocated) => {}
            other => panic!("expected RE entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_pn_entry() {
        // PN entry: header(4) + dev_high(8) + dev_low(8) = 20
        let mut data = vec![b'P', b'N', 20, 1];
        data.extend_from_slice(&u32_lsb_msb_bytes(8)); // major
        data.extend_from_slice(&u32_lsb_msb_bytes(1)); // minor

        let mut iter = SystemUseIter::new(&data, 0);
        match iter.next() {
            Some(SystemUseField::DeviceNumber(pn)) => {
                assert_eq!(pn.dev_high.read(), 8);
                assert_eq!(pn.dev_low.read(), 1);
            }
            other => panic!("expected PN entry, got {:?}", other),
        }
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_mixed_susp_rrip() {
        let mut data = Vec::new();
        // SP entry
        data.extend_from_slice(&[b'S', b'P', 7, 1, 0xBE, 0xEF, 0]);
        // PX entry (44 bytes)
        data.extend_from_slice(&[b'P', b'X', 44, 1]);
        data.extend_from_slice(&u32_lsb_msb_bytes(0o040755)); // dir mode
        data.extend_from_slice(&u32_lsb_msb_bytes(2));
        data.extend_from_slice(&u32_lsb_msb_bytes(0));
        data.extend_from_slice(&u32_lsb_msb_bytes(0));
        data.extend_from_slice(&u32_lsb_msb_bytes(1));
        // NM entry for "mydir"
        data.extend_from_slice(&[b'N', b'M', 10, 1, 0x00]);
        data.extend_from_slice(b"mydir");
        // TF entry with MODIFY flag (7 bytes timestamp)
        data.extend_from_slice(&[b'T', b'F', 12, 1, 0x02]);
        data.extend_from_slice(&[126, 2, 16, 8, 0, 0, 0]);
        // ST terminator
        data.extend_from_slice(&[b'S', b'T', 4, 1]);

        let fields: Vec<_> = SystemUseIter::new(&data, 0).collect();
        assert_eq!(fields.len(), 5);
        assert!(matches!(fields[0], SystemUseField::SuspIdentifier(_)));
        assert!(matches!(fields[1], SystemUseField::PosixAttributes(_)));
        assert!(matches!(fields[2], SystemUseField::AlternateName(_)));
        assert!(matches!(fields[3], SystemUseField::Timestamps(_)));
        assert!(matches!(fields[4], SystemUseField::Terminator));

        // Verify signature() method
        assert_eq!(fields[0].signature(), *b"SP");
        assert_eq!(fields[1].signature(), *b"PX");
        assert_eq!(fields[2].signature(), *b"NM");
        assert_eq!(fields[3].signature(), *b"TF");
        assert_eq!(fields[4].signature(), *b"ST");
    }
}

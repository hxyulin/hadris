/// Path Table record header (ECMA-119 9.4).
///
/// The same layout serves both tables: the little-endian (type L) table
/// stores `extent` and `parent` little-endian, the big-endian (type M) table
/// big-endian. The directory identifier follows, padded to an even length.
///
/// @hadris-spec ECMA-119:9.4
/// @hadris-compliance partial
/// @hadris-note Both L- and M-type path tables are written and read; the optional secondary path tables are not populated.
/// @hadris-tests iso::spec::hadris_iso_matches_ecma_119_oracle
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathTableHeader {
    /// The length of the directory identifier.
    pub len: u8,
    /// The length of the extended attribute record.
    pub extended_attr_record: u8,
    /// The first logical block of the directory.
    pub extent: [u8; 4],
    /// The number of the parent directory's record, from 1.
    pub parent: [u8; 2],
}

impl PathTableHeader {
    /// A little-endian (type L) header.
    pub const fn little(len: u8, extent: u32, parent: u16) -> Self {
        Self {
            len,
            extended_attr_record: 0,
            extent: extent.to_le_bytes(),
            parent: parent.to_le_bytes(),
        }
    }

    /// A big-endian (type M) header.
    pub const fn big(len: u8, extent: u32, parent: u16) -> Self {
        Self {
            len,
            extended_attr_record: 0,
            extent: extent.to_be_bytes(),
            parent: parent.to_be_bytes(),
        }
    }

    /// The directory's extent in a type L table.
    pub const fn extent_le(&self) -> u32 {
        u32::from_le_bytes(self.extent)
    }

    /// The directory's extent in a type M table.
    pub const fn extent_be(&self) -> u32 {
        u32::from_be_bytes(self.extent)
    }

    /// The parent's record number in a type L table.
    pub const fn parent_le(&self) -> u16 {
        u16::from_le_bytes(self.parent)
    }

    /// The parent's record number in a type M table.
    pub const fn parent_be(&self) -> u16 {
        u16::from_be_bytes(self.parent)
    }

    /// The size of the whole record: header, identifier and padding.
    pub const fn record_len(&self) -> usize {
        (8 + self.len as usize + 1) & !1
    }
}

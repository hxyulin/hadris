use hadris_fixed::FixedBytes;

use super::io::{self, Error, LogicalSector, Read, Write};
use crate::types::EndianType;

sync_only! {
#[cfg(feature = "alloc")]
use core::ops::DerefMut;
#[cfg(feature = "alloc")]
use spin::Mutex;
#[cfg(feature = "alloc")]
use super::io::{Seek, SeekFrom};
#[cfg(feature = "alloc")]
use super::read::IsoImage;
}

/// Path Table record header (ECMA-119 9.4).
///
/// @hadris-spec ECMA-119:9.4
/// @hadris-compliance partial
/// @hadris-note Both L- and M-type path tables are written and read; the optional secondary path tables are not populated.
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathTableEntryHeader {
    /// The `len` field.
    pub len: u8,
    /// The `extended_attr_record` field.
    pub extended_attr_record: u8,
    /// The `parent_lba` field.
    pub parent_lba: [u8; 4],
    /// The `parent_directory_number` field.
    pub parent_directory_number: [u8; 2],
}

impl PathTableEntryHeader {
    /// Performs the `from_bytes` operation.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

#[derive(Debug, Clone)]
/// Represents PathTableEntry.
pub struct PathTableEntry<const N: usize = 256> {
    /// The `length` field.
    pub length: u8,
    /// The `extended_attr_record` field.
    pub extended_attr_record: u8,
    /// The `parent_lba` field.
    pub parent_lba: u32,
    /// The `parent_index` field.
    pub parent_index: u16,
    /// The `name` field.
    pub name: FixedBytes<N>,
}

impl PathTableEntry {
    /// Performs the `size` operation.
    pub fn size(&self) -> usize {
        (size_of::<PathTableEntryHeader>() + self.name.len() + 1) & !1
    }
}

io_transform! {
impl PathTableEntry {
    /// Performs the `parse` operation.
    pub async fn parse<T: Read>(reader: &mut T, endian: EndianType) -> Result<Self, Error> {
        let mut buf = [0; size_of::<PathTableEntryHeader>()];
        reader.read_exact(&mut buf).await?;
        let header = PathTableEntryHeader::from_bytes(&buf);
        let mut name = FixedBytes::with_size(header.len as usize);
        reader.read_exact(name.as_bytes_mut()).await?;
        if header.len % 2 == 1 {
            // Read the padding byte
            reader.read_exact(&mut [0]).await?;
        }

        Ok(Self {
            length: header.len,
            extended_attr_record: header.extended_attr_record,
            parent_lba: endian.read_u32(header.parent_lba),
            parent_index: endian.read_u16(header.parent_directory_number),
            name,
        })
    }

    /// Performs the `write` operation.
    pub async fn write<W: Write>(&self, writer: &mut W, endian: EndianType) -> io::Result<()> {
        let header = PathTableEntryHeader {
            len: self.name.len() as u8,
            extended_attr_record: 0,
            parent_lba: endian.u32_bytes(self.parent_lba),
            parent_directory_number: endian.u16_bytes(self.parent_index),
        };
        writer.write_all(bytemuck::bytes_of(&header)).await?;
        writer.write_all(self.name.as_bytes()).await?;
        assert_eq!(header.len as usize, self.name.len());
        if header.len % 2 == 1 {
            writer.write_all(&[0]).await?;
        }
        Ok(())
    }
}
} // io_transform!

#[derive(Debug, Clone, Copy)]
/// Represents PathTableRef.
pub struct PathTableRef {
    pub(crate) lpt: LogicalSector,
    pub(crate) mpt: LogicalSector,
    pub(crate) size: u64,
}

/// Path table information (requires alloc for iterator support)
#[cfg(feature = "alloc")]
pub struct PathTableInfo {
    pub(crate) path_table: PathTableRef,
}

#[cfg(feature = "alloc")]
impl PathTableInfo {
    /// Returns the underlying path-table locations and encoded length.
    pub fn reference(&self) -> &PathTableRef {
        &self.path_table
    }
}

impl PathTableRef {
    /// Returns the little-endian path-table sector.
    pub fn little_endian_sector(&self) -> LogicalSector {
        self.lpt
    }

    /// Returns the big-endian path-table sector.
    pub fn big_endian_sector(&self) -> LogicalSector {
        self.mpt
    }

    /// Returns the encoded path-table length in bytes.
    pub fn len(&self) -> u64 {
        self.size
    }

    /// Returns whether the encoded path table is empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

sync_only! {
#[cfg(feature = "alloc")]
impl PathTableInfo {
    /// Performs the `entries` operation.
    pub fn entries<'a, DATA: Read + Seek>(
        &self,
        image: &'a IsoImage<DATA>,
    ) -> PathTableEntryIter<'a, DATA> {
        let start = if cfg!(target_endian = "little") {
            self.path_table.lpt
        } else {
            self.path_table.mpt
        };
        // Path table starts at the given sector, convert to byte offset
        let start_byte = (start.0 as u64) * 2048;
        let end_byte = start_byte + self.path_table.size;
        PathTableEntryIter {
            data: &image.data,
            current: start_byte,
            end: end_byte,
        }
    }
}

#[cfg(feature = "alloc")]
/// Represents PathTableEntryIter.
pub struct PathTableEntryIter<'a, DATA: Read + Seek> {
    data: &'a Mutex<super::io::IsoCursor<DATA>>,
    current: u64,
    end: u64,
}

#[cfg(feature = "alloc")]
impl<DATA: Read + Seek> Iterator for PathTableEntryIter<'_, DATA> {
    type Item = io::Result<PathTableEntry>;

    /// Undefined if continued reading after IO error
    fn next(&mut self) -> Option<Self::Item> {
        use super::io::try_io_result_option as try_io;
        if self.current >= self.end {
            return None;
        }
        let mut data = self.data.lock();
        try_io!(data
            .seek(SeekFrom::Start(self.current))
            .map_err(Error::erase));
        let entry = try_io!(PathTableEntry::parse(
            data.deref_mut(),
            EndianType::NativeEndian,
        ));
        self.current += entry.size() as u64;

        Some(Ok(entry))
    }
}
} // sync_only!

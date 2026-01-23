use core::ops::DerefMut;

use hadris_common::types::file::FixedFilename;
use spin::Mutex;

use crate::{
    io::{self, Error, LogicalSector, Read, Seek, SeekFrom, Write},
    types::EndianType,
};

#[cfg(feature = "alloc")]
use crate::read::IsoImage;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathTableEntryHeader {
    pub len: u8,
    pub extended_attr_record: u8,
    pub parent_lba: [u8; 4],
    pub parent_directory_number: [u8; 2],
}

impl PathTableEntryHeader {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

#[derive(Debug, Clone)]
pub struct PathTableEntry<const N: usize = 256> {
    pub length: u8,
    pub extended_attr_record: u8,
    pub parent_lba: u32,
    pub parent_index: u16,
    pub name: FixedFilename<N>,
}

impl PathTableEntry {
    pub fn parse<T: Read>(reader: &mut T, endian: EndianType) -> Result<Self, Error> {
        let mut buf = [0; size_of::<PathTableEntryHeader>()];
        reader.read_exact(&mut buf)?;
        let header = PathTableEntryHeader::from_bytes(&buf);
        let mut name = FixedFilename::with_size(header.len as usize);
        reader.read_exact(name.as_bytes_mut())?;
        if header.len % 2 == 1 {
            // Read the padding byte
            reader.read_exact(&mut [0])?;
        }

        Ok(Self {
            length: header.len,
            extended_attr_record: header.extended_attr_record,
            parent_lba: endian.read_u32(header.parent_lba),
            parent_index: endian.read_u16(header.parent_directory_number),
            name,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W, endian: EndianType) -> io::Result<()> {
        let header = PathTableEntryHeader {
            len: self.name.len() as u8,
            extended_attr_record: 0,
            parent_lba: endian.u32_bytes(self.parent_lba),
            parent_directory_number: endian.u16_bytes(self.parent_index),
        };
        writer.write_all(bytemuck::bytes_of(&header))?;
        writer.write_all(self.name.as_bytes())?;
        assert_eq!(header.len as usize, self.name.len());
        if header.len % 2 == 1 {
            writer.write_all(&[0])?;
        }
        Ok(())
    }

    pub fn size(&self) -> usize {
        let size = (size_of::<PathTableEntryHeader>() + self.name.len() + 1) & !1;
        size
    }
}

#[derive(Debug, Clone, Copy)]
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
pub struct PathTableEntryIter<'a, DATA: Read + Seek> {
    data: &'a Mutex<crate::io::IsoCursor<DATA>>,
    current: u64,
    end: u64,
}

#[cfg(feature = "alloc")]
impl<DATA: Read + Seek> Iterator for PathTableEntryIter<'_, DATA> {
    type Item = io::Result<PathTableEntry>;

    /// Undefined if continued reading after IO error
    fn next(&mut self) -> Option<Self::Item> {
        use hadris_io::try_io_result_option as try_io;
        if self.current >= self.end {
            return None;
        }
        let mut data = self.data.lock();
        try_io!(data.seek(SeekFrom::Start(self.current)));
        let entry = try_io!(PathTableEntry::parse(
            data.deref_mut(),
            EndianType::NativeEndian,
        ));
        self.current += entry.size() as u64;

        Some(Ok(entry))
    }
}

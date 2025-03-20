use std::io::Read;

use crate::types::EndianType;

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
pub struct PathTableEntry {
    pub length: u8,
    pub extended_attr_record: u8,
    pub parent_lba: u32,
    pub parent_index: u16,
    pub name: String,
}

impl PathTableEntry {
    pub fn parse<T: Read>(reader: &mut T, endian: EndianType) -> Result<Self, std::io::Error> {
        let mut buf = [0; size_of::<PathTableEntryHeader>()];
        reader.read_exact(&mut buf)?;
        let header = PathTableEntryHeader::from_bytes(&buf);
        let mut name = vec![0; header.len as usize];
        reader.read_exact(&mut name)?;
        if header.len % 2 == 1 {
            // Read the padding byte
            reader.read_exact(&mut [0])?;
        }

        Ok(Self {
            length: header.len,
            extended_attr_record: header.extended_attr_record,
            parent_lba: endian.read_u32(header.parent_lba),
            parent_index: endian.read_u16(header.parent_directory_number),
            name: String::from_utf8(name).unwrap(),
        })
    }

    pub fn to_bytes(&self, endian: EndianType) -> Vec<u8> {
        let mut bytes = Vec::new();
        let header = PathTableEntryHeader {
            len: self.name.len() as u8,
            extended_attr_record: 0,
            parent_lba: endian.u32_bytes(self.parent_lba),
            parent_directory_number: endian.u16_bytes(self.parent_index),
        };
        bytes.extend_from_slice(bytemuck::bytes_of(&header));
        bytes.extend_from_slice(self.name.as_bytes());
        assert_eq!(header.len as usize, self.name.len());
        if header.len % 2 == 1 {
            bytes.push(0);
        }

        bytes
    }
    pub fn size(&self) -> usize {
        let size = (size_of::<PathTableEntryHeader>() + self.name.len() + 1) & !1;
        size
    }
}

use std::{
    ffi::CStr,
    fmt::Debug,
    io::{Read, Write},
};

use crate::{
    directory::RootDirectoryEntry,
    types::{
        BigEndian, DecDateTime, Endian, IsoStrA, IsoStrD, LittleEndian, U16LsbMsb, U32, U32LsbMsb,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeDescriptorType {
    BootRecord,
    PrimaryVolumeDescriptor,
    SupplementaryVolumeDescriptor,
    VolumePartitionDescriptor,
    VolumeSetTerminator,
    Unknown(u8),
}

impl VolumeDescriptorType {
    pub fn to_u8(self) -> u8 {
        match self {
            Self::BootRecord => 0x00,
            Self::PrimaryVolumeDescriptor => 0x01,
            Self::SupplementaryVolumeDescriptor => 0x02,
            Self::VolumePartitionDescriptor => 0x03,
            Self::VolumeSetTerminator => 0xFF,
            Self::Unknown(value) => value,
        }
    }
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::BootRecord,
            0x01 => Self::PrimaryVolumeDescriptor,
            0x02 => Self::SupplementaryVolumeDescriptor,
            0x03 => Self::VolumePartitionDescriptor,
            0xFF => Self::VolumeSetTerminator,
            value => Self::Unknown(value),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VolumeDescriptor {
    BootRecord(BootRecordVolumeDescriptor),
    Primary(PrimaryVolumeDescriptor),
    End(VolumeDescriptorSetTerminator),
    Unknown(UnknownVolumeDescriptor),
}

impl VolumeDescriptor {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            VolumeDescriptor::BootRecord(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::Primary(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::End(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::Unknown(entry) => bytemuck::bytes_of(entry),
        }
    }

    pub fn to_bytes(&self) -> &[u8] {
        match self {
            VolumeDescriptor::BootRecord(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::Primary(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::End(entry) => bytemuck::bytes_of(entry),
            VolumeDescriptor::Unknown(entry) => bytemuck::bytes_of(entry),
        }
    }

    pub fn header(&self) -> VolumeDescriptorHeader {
        match self {
            VolumeDescriptor::BootRecord(entry) => entry.header,
            VolumeDescriptor::Primary(entry) => entry.header,
            VolumeDescriptor::End(entry) => entry.header,
            VolumeDescriptor::Unknown(entry) => entry.header,
        }
    }

    pub fn new(data: &[u8]) -> Self {
        assert!(data.len() == 2048);
        let ty = VolumeDescriptorType::from_u8(data[0]);
        match ty {
            VolumeDescriptorType::BootRecord => {
                VolumeDescriptor::BootRecord(*bytemuck::from_bytes(data))
            }
            VolumeDescriptorType::PrimaryVolumeDescriptor => {
                VolumeDescriptor::Primary(*bytemuck::from_bytes(data))
            }
            VolumeDescriptorType::VolumeSetTerminator => {
                VolumeDescriptor::End(*bytemuck::from_bytes(data))
            }
            _ => VolumeDescriptor::Unknown(*bytemuck::from_bytes(data)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeDescriptorList {
    pub descriptors: Vec<VolumeDescriptor>,
}

impl VolumeDescriptorList {
    pub fn empty() -> Self {
        Self {
            descriptors: Vec::new(),
        }
    }

    /// Parse the volume descriptor list from the given reader
    ///
    /// The caller should seek to the start of the volume descriptor list, which is usually at LBA 16
    pub fn parse<T: Read>(reader: &mut T) -> Result<Self, std::io::Error> {
        let mut descriptors = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            reader.read_exact(&mut buffer)?;
            let header = VolumeDescriptorHeader::from_bytes(&buffer[0..7]);
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);
            if let VolumeDescriptorType::VolumeSetTerminator = ty {
                break;
            }
            if !header.is_valid() {
                // Invalid, which means either we are at the wrong place, or the writer didn't
                // write an end record
                panic!("Invalid volume descriptor header, did you forget to seek to LBA 16?");
            }

            descriptors.push(VolumeDescriptor::new(&buffer));
        }

        Ok(Self { descriptors })
    }

    pub fn primary(&self) -> &PrimaryVolumeDescriptor {
        self.descriptors
            .iter()
            .find_map(|d| match d {
                VolumeDescriptor::Primary(d) => Some(d),
                _ => None,
            })
            .expect("Primary volume descriptor not found")
    }

    pub fn primary_mut(&mut self) -> &mut PrimaryVolumeDescriptor {
        self.descriptors
            .iter_mut()
            .find_map(|d| match d {
                VolumeDescriptor::Primary(d) => Some(d),
                _ => None,
            })
            .expect("Primary volume descriptor not found")
    }

    pub fn boot_record(&self) -> Option<&BootRecordVolumeDescriptor> {
        self.descriptors.iter().find_map(|d| match d {
            VolumeDescriptor::BootRecord(d) => Some(d),
            _ => None,
        })
    }

    pub fn boot_record_mut(&mut self) -> Option<&mut BootRecordVolumeDescriptor> {
        self.descriptors.iter_mut().find_map(|d| match d {
            VolumeDescriptor::BootRecord(d) => Some(d),
            _ => None,
        })
    }

    pub fn push(&mut self, descriptor: VolumeDescriptor) {
        self.descriptors.push(descriptor);
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<usize, std::io::Error> {
        let mut written = 0;
        for descriptor in &self.descriptors {
            writer.write_all(&descriptor.to_bytes())?;
            written += 2048;
        }
        writer.write_all(VolumeDescriptorSetTerminator::new().to_bytes())?;
        written += 2048;
        Ok(written)
    }

    pub fn size_required(&self) -> usize {
        (self.descriptors.len() + 1) * 2048
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumeDescriptorHeader {
    pub descriptor_type: u8,
    pub standard_identifier: IsoStrA<5>,
    pub version: u8,
}

impl Debug for VolumeDescriptorHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VolumeDescriptorHeader")
            .field(
                "descriptor_type",
                &VolumeDescriptorType::from_u8(self.descriptor_type),
            )
            .field("standard_identifier", &self.standard_identifier)
            .field("version", &self.version)
            .finish()
    }
}

impl VolumeDescriptorHeader {
    const IDENTIFIER: IsoStrA<5> = IsoStrA::from_bytes_exact(*b"CD001");
    pub fn new(ty: VolumeDescriptorType) -> Self {
        Self {
            descriptor_type: ty.to_u8(),
            standard_identifier: Self::IDENTIFIER,
            version: 1,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.standard_identifier == Self::IDENTIFIER
    }

    pub fn from_bytes(bytes: &[u8]) -> &Self {
        bytemuck::from_bytes(bytes)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UnknownVolumeDescriptor {
    header: VolumeDescriptorHeader,
    data: [u8; 2041],
}

impl Debug for UnknownVolumeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnknownVolumeDescriptor")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

unsafe impl bytemuck::Zeroable for UnknownVolumeDescriptor {}
unsafe impl bytemuck::Pod for UnknownVolumeDescriptor {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PrimaryVolumeDescriptor {
    pub header: VolumeDescriptorHeader,
    pub unused0: u8,
    pub system_identifier: IsoStrA<32>,
    pub volume_identifier: IsoStrD<32>,
    pub unused1: [u8; 8],
    pub volume_space_size: U32LsbMsb,
    pub unused2: [u8; 32],
    pub volume_set_size: U16LsbMsb,
    pub volume_sequence_number: U16LsbMsb,
    pub logical_block_size: U16LsbMsb,
    pub path_table_size: U32LsbMsb,
    pub type_l_path_table: U32<LittleEndian>,
    pub opt_type_l_path_table: U32<LittleEndian>,
    pub type_m_path_table: U32<BigEndian>,
    pub opt_type_m_path_table: U32<BigEndian>,
    pub dir_record: RootDirectoryEntry,
    pub volume_set_identifier: IsoStrD<128>,
    pub publisher_identifier: IsoStrA<128>,
    pub preparer_identifier: IsoStrA<128>,
    pub application_identifier: IsoStrA<128>,
    pub copyright_file_identifier: IsoStrD<37>,
    pub abstract_file_identifier: IsoStrD<37>,
    pub bibliographic_file_identifier: IsoStrD<37>,
    pub creation_date: DecDateTime,
    pub modification_date: DecDateTime,
    pub expiration_date: DecDateTime,
    pub effective_date: DecDateTime,
    pub file_structure_version: u8,
    pub unused3: u8,
    pub app_data: [u8; 512],
    pub reserved: [u8; 653],
}

impl Debug for PrimaryVolumeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrimaryVolumeDescriptor")
            .field("header", &self.header)
            .field("system_identifier", &self.system_identifier)
            .field("volume_identifier", &self.volume_identifier)
            .field("volume_space_size", &self.volume_space_size)
            .field("volume_set_size", &self.volume_set_size)
            .field("volume_sequence_number", &self.volume_sequence_number)
            .field("logical_block_size", &self.logical_block_size)
            .field("path_table_size", &self.path_table_size)
            .field("type_l_path_table", &self.type_l_path_table)
            .field("opt_type_l_path_table", &self.opt_type_l_path_table)
            .field("type_m_path_table", &self.type_m_path_table)
            .field("opt_type_m_path_table", &self.opt_type_m_path_table)
            .field("dir_record", &self.dir_record)
            .field("volume_set_identifier", &self.volume_set_identifier)
            .field("publisher_identifier", &self.publisher_identifier)
            .field("preparer_identifier", &self.preparer_identifier)
            .field("application_identifier", &self.application_identifier)
            .field("copyright_file_identifier", &self.copyright_file_identifier)
            .field("abstract_file_identifier", &self.abstract_file_identifier)
            .field(
                "bibliographic_file_identifier",
                &self.bibliographic_file_identifier,
            )
            .field("creation_date", &self.creation_date)
            .field("modification_date", &self.modification_date)
            .field("expiration_date", &self.expiration_date)
            .field("effective_date", &self.effective_date)
            .field("file_structure_version", &self.file_structure_version)
            .finish_non_exhaustive()
    }
}

impl PrimaryVolumeDescriptor {
    pub fn new(name: &str, sectors: u32) -> Self {
        Self {
            header: VolumeDescriptorHeader {
                descriptor_type: VolumeDescriptorType::PrimaryVolumeDescriptor.to_u8(),
                standard_identifier: IsoStrA::from_str("CD001").unwrap(),
                version: 1,
            },
            unused0: 0,
            system_identifier: IsoStrA::empty(),
            volume_identifier: IsoStrD::from_str(name).unwrap(),
            unused1: [0; 8],
            volume_space_size: U32LsbMsb::new(sectors),
            unused2: [0; 32],
            volume_set_size: U16LsbMsb::new(1),
            volume_sequence_number: U16LsbMsb::new(1),
            logical_block_size: U16LsbMsb::new(2048),
            path_table_size: U32LsbMsb::new(0),
            type_l_path_table: U32::<LittleEndian>::new(0),
            opt_type_l_path_table: U32::<LittleEndian>::new(0),
            type_m_path_table: U32::<BigEndian>::new(0),
            opt_type_m_path_table: U32::<BigEndian>::new(0),
            dir_record: RootDirectoryEntry::default(),
            volume_set_identifier: IsoStrD::empty(),
            publisher_identifier: IsoStrA::empty(),
            preparer_identifier: IsoStrA::empty(),
            application_identifier: IsoStrA::from_str("ISO-RS").unwrap(),
            copyright_file_identifier: IsoStrD::empty(),
            abstract_file_identifier: IsoStrD::empty(),
            bibliographic_file_identifier: IsoStrD::empty(),
            creation_date: DecDateTime::now(),
            modification_date: DecDateTime::now(),
            expiration_date: DecDateTime::now(),
            effective_date: DecDateTime::now(),
            file_structure_version: 1,
            unused3: 0,
            app_data: [0; 512],
            reserved: [0; 653],
        }
    }
}

unsafe impl bytemuck::Zeroable for PrimaryVolumeDescriptor {}
unsafe impl bytemuck::Pod for PrimaryVolumeDescriptor {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BootRecordVolumeDescriptor {
    pub header: VolumeDescriptorHeader,
    pub boot_system_identifier: [u8; 32],
    pub unused0: [u8; 32],
    pub catalog_ptr: U32<LittleEndian>,
    pub unused1: [u8; 1973],
}

impl BootRecordVolumeDescriptor {
    pub fn new(catalog_sector: u32) -> Self {
        const BOOT_SYSTEM_IDENTIFIER: [u8; 32] = *b"EL TORITO SPECIFICATION\0\0\0\0\0\0\0\0\0";
        Self {
            header: VolumeDescriptorHeader::new(VolumeDescriptorType::BootRecord),
            boot_system_identifier: BOOT_SYSTEM_IDENTIFIER,
            unused0: [0; 32],
            catalog_ptr: U32::<LittleEndian>::new(catalog_sector),
            unused1: [0; 1973],
        }
    }
}

impl Debug for BootRecordVolumeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let system_identifier = CStr::from_bytes_until_nul(&self.boot_system_identifier);
        f.debug_struct("BootRecordVolumeDescriptor")
            .field("header", &self.header)
            .field("boot_system_identifier", &system_identifier)
            .field("catalog_ptr", &self.catalog_ptr)
            .finish_non_exhaustive()
    }
}

unsafe impl bytemuck::Zeroable for BootRecordVolumeDescriptor {}
unsafe impl bytemuck::Pod for BootRecordVolumeDescriptor {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VolumeDescriptorSetTerminator {
    header: VolumeDescriptorHeader,
    padding: [u8; 2041],
}

impl VolumeDescriptorSetTerminator {
    pub fn new() -> Self {
        Self {
            header: VolumeDescriptorHeader {
                descriptor_type: VolumeDescriptorType::VolumeSetTerminator.to_u8(),
                standard_identifier: IsoStrA::empty(),
                version: 1,
            },
            padding: [0; 2041],
        }
    }

    pub fn to_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

impl Debug for VolumeDescriptorSetTerminator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VolumeDescriptorSetTerminator")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

unsafe impl bytemuck::Zeroable for VolumeDescriptorSetTerminator {}
unsafe impl bytemuck::Pod for VolumeDescriptorSetTerminator {}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_eq_size!(PrimaryVolumeDescriptor, [u8; 2048]);
    static_assertions::assert_eq_size!(VolumeDescriptorSetTerminator, [u8; 2048]);
    static_assertions::assert_eq_size!(BootRecordVolumeDescriptor, [u8; 2048]);
    static_assertions::assert_eq_size!(UnknownVolumeDescriptor, [u8; 2048]);

    static_assertions::assert_eq_align!(PrimaryVolumeDescriptor, u8);
    static_assertions::assert_eq_align!(VolumeDescriptorSetTerminator, u8);
    static_assertions::assert_eq_align!(BootRecordVolumeDescriptor, u8);
}

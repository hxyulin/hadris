use core::fmt;

use super::{
    DecDateTime, IsoStr, RootDirectoryRecord, SECTOR_SIZE, U16Both, U32Be, U32Both, U32Le,
};

/// The standard identifier every volume descriptor carries.
pub const STANDARD_ID: [u8; 5] = *b"CD001";

/// The boot system identifier of an El Torito boot record.
pub const EL_TORITO_ID: [u8; 32] = *b"EL TORITO SPECIFICATION\0\0\0\0\0\0\0\0\0";

/// The type byte of a volume descriptor (ECMA-119 8.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorType {
    /// 0: a boot record.
    BootRecord,
    /// 1: the primary volume descriptor.
    Primary,
    /// 2: a supplementary or enhanced volume descriptor.
    Supplementary,
    /// 3: a volume partition descriptor.
    Partition,
    /// 255: the set terminator.
    Terminator,
    /// Any other value.
    Other(u8),
}

impl DescriptorType {
    /// Decodes the type byte.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::BootRecord,
            1 => Self::Primary,
            2 => Self::Supplementary,
            3 => Self::Partition,
            255 => Self::Terminator,
            other => Self::Other(other),
        }
    }

    /// Encodes the type byte.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::BootRecord => 0,
            Self::Primary => 1,
            Self::Supplementary => 2,
            Self::Partition => 3,
            Self::Terminator => 255,
            Self::Other(other) => other,
        }
    }
}

/// The first seven bytes of every volume descriptor (ECMA-119 8.1).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumeDescriptorHeader {
    /// The descriptor type.
    pub descriptor_type: u8,
    /// `CD001`.
    pub standard_identifier: [u8; 5],
    /// The descriptor version: 1, or 2 for an enhanced volume descriptor.
    pub version: u8,
}

impl VolumeDescriptorHeader {
    /// A header of type `ty`, version 1.
    pub const fn new(ty: DescriptorType) -> Self {
        Self {
            descriptor_type: ty.to_u8(),
            standard_identifier: STANDARD_ID,
            version: 1,
        }
    }

    /// The descriptor type.
    pub const fn kind(&self) -> DescriptorType {
        DescriptorType::from_u8(self.descriptor_type)
    }

    /// Whether the identifier is `CD001` and the version is 1, or 2 for a
    /// supplementary descriptor.
    pub fn is_valid(&self) -> bool {
        self.standard_identifier == STANDARD_ID
            && (self.version == 1
                || (self.descriptor_type == DescriptorType::Supplementary.to_u8()
                    && self.version == 2))
    }
}

impl fmt::Debug for VolumeDescriptorHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VolumeDescriptorHeader")
            .field("descriptor_type", &self.kind())
            .field("standard_identifier", &self.standard_identifier)
            .field("version", &self.version)
            .finish()
    }
}

/// Primary Volume Descriptor (ECMA-119 8.4)
///
/// @hadris-spec ECMA-119:8.4
/// @hadris-compliance partial
/// @hadris-note Core fields are modeled, but reserved fields, character sets, redundant endian values, and semantic constraints are not all validated.
/// @hadris-tests read::descriptor_sequence_opens_primary_volume_and_root_directory, iso::spec::hadris_iso_matches_ecma_119_oracle
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimaryVolumeDescriptor {
    /// Type 1, `CD001`, version 1.
    pub header: VolumeDescriptorHeader,
    /// Unused, zero.
    pub unused0: u8,
    /// The system that may use sectors 0 to 15.
    pub system_identifier: IsoStr<32>,
    /// The volume name.
    pub volume_identifier: IsoStr<32>,
    /// Unused, zero.
    pub unused1: [u8; 8],
    /// The number of logical blocks in the volume.
    pub volume_space_size: U32Both,
    /// Unused, zero. The escape sequences of a supplementary descriptor.
    pub unused2: [u8; 32],
    /// The number of volumes in the set.
    pub volume_set_size: U16Both,
    /// This volume's number in the set.
    pub volume_sequence_number: U16Both,
    /// The logical block size, usually 2048.
    pub logical_block_size: U16Both,
    /// The size of each path table in bytes.
    pub path_table_size: U32Both,
    /// The location of the little-endian path table.
    pub type_l_path_table: U32Le,
    /// The location of the optional little-endian path table.
    pub opt_type_l_path_table: U32Le,
    /// The location of the big-endian path table.
    pub type_m_path_table: U32Be,
    /// The location of the optional big-endian path table.
    pub opt_type_m_path_table: U32Be,
    /// The root directory record.
    pub root: RootDirectoryRecord,
    /// The volume set name.
    pub volume_set_identifier: IsoStr<128>,
    /// The publisher.
    pub publisher_identifier: IsoStr<128>,
    /// The data preparer.
    pub preparer_identifier: IsoStr<128>,
    /// The application that wrote the volume.
    pub application_identifier: IsoStr<128>,
    /// The copyright file in the root directory.
    pub copyright_file_identifier: IsoStr<37>,
    /// The abstract file in the root directory.
    pub abstract_file_identifier: IsoStr<37>,
    /// The bibliographic file in the root directory.
    pub bibliographic_file_identifier: IsoStr<37>,
    /// When the volume was created.
    pub creation_date: DecDateTime,
    /// When the volume was last modified.
    pub modification_date: DecDateTime,
    /// When the volume becomes obsolete.
    pub expiration_date: DecDateTime,
    /// When the volume may be used.
    pub effective_date: DecDateTime,
    /// 1, or 2 for an enhanced volume descriptor.
    pub file_structure_version: u8,
    /// Reserved, zero.
    pub unused3: u8,
    /// Application use.
    pub app_data: [u8; 512],
    /// Reserved, zero.
    pub reserved: [u8; 653],
}

/// Supplementary or enhanced volume descriptor (ECMA-119 8.5), used for the
/// Joliet namespace and the ISO 9660:1999 enhanced tree.
///
/// It has the layout of the primary descriptor; the `flags` and
/// `escape_sequences` fields take the place of unused bytes.
///
/// @hadris-spec ECMA-119:8.5
/// @hadris-compliance partial
/// @hadris-note Joliet SVDs (UCS-2, BMP only) and version-2 enhanced descriptors are read and written; the escape sequences recognized are the three Joliet levels.
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SupplementaryVolumeDescriptor {
    /// Type 2, `CD001`, version 1, or version 2 for an enhanced descriptor.
    pub header: VolumeDescriptorHeader,
    /// Volume flags.
    pub flags: u8,
    /// The system that may use sectors 0 to 15.
    pub system_identifier: IsoStr<32>,
    /// The volume name.
    pub volume_identifier: IsoStr<32>,
    /// Unused, zero.
    pub unused1: [u8; 8],
    /// The number of logical blocks in the volume.
    pub volume_space_size: U32Both,
    /// The character set escape sequences; Joliet stores `%/@`, `%/C` or
    /// `%/E` here.
    pub escape_sequences: [u8; 32],
    /// The number of volumes in the set.
    pub volume_set_size: U16Both,
    /// This volume's number in the set.
    pub volume_sequence_number: U16Both,
    /// The logical block size.
    pub logical_block_size: U16Both,
    /// The size of each path table in bytes.
    pub path_table_size: U32Both,
    /// The location of the little-endian path table.
    pub type_l_path_table: U32Le,
    /// The location of the optional little-endian path table.
    pub opt_type_l_path_table: U32Le,
    /// The location of the big-endian path table.
    pub type_m_path_table: U32Be,
    /// The location of the optional big-endian path table.
    pub opt_type_m_path_table: U32Be,
    /// The root directory record.
    pub root: RootDirectoryRecord,
    /// The volume set name.
    pub volume_set_identifier: IsoStr<128>,
    /// The publisher.
    pub publisher_identifier: IsoStr<128>,
    /// The data preparer.
    pub preparer_identifier: IsoStr<128>,
    /// The application that wrote the volume.
    pub application_identifier: IsoStr<128>,
    /// The copyright file in the root directory.
    pub copyright_file_identifier: IsoStr<37>,
    /// The abstract file in the root directory.
    pub abstract_file_identifier: IsoStr<37>,
    /// The bibliographic file in the root directory.
    pub bibliographic_file_identifier: IsoStr<37>,
    /// When the volume was created.
    pub creation_date: DecDateTime,
    /// When the volume was last modified.
    pub modification_date: DecDateTime,
    /// When the volume becomes obsolete.
    pub expiration_date: DecDateTime,
    /// When the volume may be used.
    pub effective_date: DecDateTime,
    /// 1, or 2 for an enhanced volume descriptor.
    pub file_structure_version: u8,
    /// Reserved, zero.
    pub unused3: u8,
    /// Application use.
    pub app_data: [u8; 512],
    /// Reserved, zero.
    pub reserved: [u8; 653],
}

impl SupplementaryVolumeDescriptor {
    /// The Joliet level, 1 to 3, from the escape sequences.
    pub fn joliet_level(&self) -> Option<u8> {
        super::JOLIET_ESCAPES
            .iter()
            .position(|escape| self.escape_sequences[..3] == *escape)
            .map(|level| level as u8 + 1)
    }

    /// Whether this is an ISO 9660:1999 enhanced volume descriptor.
    pub fn is_enhanced(&self) -> bool {
        self.header.version == 2 && self.file_structure_version == 2
    }
}

macro_rules! descriptor_debug {
    ($ty:ident) => {
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("header", &self.header)
                    .field("volume_identifier", &self.volume_identifier)
                    .field("volume_space_size", &self.volume_space_size)
                    .field("logical_block_size", &self.logical_block_size)
                    .field("path_table_size", &self.path_table_size)
                    .field("type_l_path_table", &self.type_l_path_table)
                    .field("type_m_path_table", &self.type_m_path_table)
                    .field("root", &self.root)
                    .field("file_structure_version", &self.file_structure_version)
                    .finish_non_exhaustive()
            }
        }
    };
}

descriptor_debug!(PrimaryVolumeDescriptor);
descriptor_debug!(SupplementaryVolumeDescriptor);

/// Boot Record Volume Descriptor (ECMA-119 8.2), locating the El Torito boot
/// catalog.
///
/// @hadris-spec ECMA-119:8.2
/// @hadris-compliance partial
/// @hadris-note The descriptor locates El Torito data, but all ECMA-119 boot-record semantics are not implemented.
/// @hadris-tests iso::boot::test_hadris_multisection_boot_catalog
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootRecordVolumeDescriptor {
    /// Type 0, `CD001`, version 1.
    pub header: VolumeDescriptorHeader,
    /// `EL TORITO SPECIFICATION` for El Torito.
    pub boot_system_identifier: [u8; 32],
    /// Unused, zero.
    pub boot_identifier: [u8; 32],
    /// The logical sector of the boot catalog.
    pub catalog_ptr: U32Le,
    /// Unused, zero.
    pub unused: [u8; 1973],
}

impl BootRecordVolumeDescriptor {
    /// An El Torito boot record pointing at `catalog_sector`.
    pub fn el_torito(catalog_sector: u32) -> Self {
        Self {
            header: VolumeDescriptorHeader::new(DescriptorType::BootRecord),
            boot_system_identifier: EL_TORITO_ID,
            boot_identifier: [0; 32],
            catalog_ptr: U32Le::new(catalog_sector),
            unused: [0; 1973],
        }
    }

    /// Whether the boot system identifier names El Torito.
    pub fn is_el_torito(&self) -> bool {
        self.boot_system_identifier == EL_TORITO_ID
    }
}

impl fmt::Debug for BootRecordVolumeDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootRecordVolumeDescriptor")
            .field("header", &self.header)
            .field("el_torito", &self.is_el_torito())
            .field("catalog_ptr", &self.catalog_ptr)
            .finish_non_exhaustive()
    }
}

/// Volume Descriptor Set Terminator (ECMA-119 8.3).
///
/// @hadris-spec ECMA-119:8.3
/// @hadris-compliance partial
/// @hadris-note The descriptor is emitted and recognized, and its body must be zero; the audit has not established validation of every other rule.
/// @hadris-tests read::malformed_primary_descriptor_and_terminator_cases_are_rejected
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumeDescriptorSetTerminator {
    /// Type 255, `CD001`, version 1.
    pub header: VolumeDescriptorHeader,
    /// Reserved, zero.
    pub reserved: [u8; 2041],
}

impl VolumeDescriptorSetTerminator {
    /// A terminator with a zero body.
    pub const fn new() -> Self {
        Self {
            header: VolumeDescriptorHeader::new(DescriptorType::Terminator),
            reserved: [0; 2041],
        }
    }
}

impl Default for VolumeDescriptorSetTerminator {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for VolumeDescriptorSetTerminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VolumeDescriptorSetTerminator")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

/// One 2048-byte volume descriptor, by type.
#[derive(Clone, Copy, Debug)]
pub enum VolumeDescriptor {
    /// A boot record.
    BootRecord(BootRecordVolumeDescriptor),
    /// The primary volume descriptor.
    Primary(PrimaryVolumeDescriptor),
    /// A supplementary or enhanced volume descriptor.
    Supplementary(SupplementaryVolumeDescriptor),
    /// The set terminator.
    Terminator(VolumeDescriptorSetTerminator),
    /// A partition descriptor or an unknown type, as raw bytes.
    Other([u8; SECTOR_SIZE]),
}

impl VolumeDescriptor {
    /// Classifies a descriptor sector by its type byte. The header is not
    /// validated; see [`VolumeDescriptorHeader::is_valid`].
    pub fn from_bytes(bytes: [u8; SECTOR_SIZE]) -> Self {
        match DescriptorType::from_u8(bytes[0]) {
            DescriptorType::BootRecord => Self::BootRecord(bytemuck::cast(bytes)),
            DescriptorType::Primary => Self::Primary(bytemuck::cast(bytes)),
            DescriptorType::Supplementary => Self::Supplementary(bytemuck::cast(bytes)),
            DescriptorType::Terminator => Self::Terminator(bytemuck::cast(bytes)),
            _ => Self::Other(bytes),
        }
    }

    /// The descriptor's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::BootRecord(d) => bytemuck::bytes_of(d),
            Self::Primary(d) => bytemuck::bytes_of(d),
            Self::Supplementary(d) => bytemuck::bytes_of(d),
            Self::Terminator(d) => bytemuck::bytes_of(d),
            Self::Other(bytes) => bytes,
        }
    }

    /// The header.
    pub fn header(&self) -> VolumeDescriptorHeader {
        *bytemuck::from_bytes(&self.as_bytes()[..7])
    }
}

const _: () = {
    assert!(size_of::<PrimaryVolumeDescriptor>() == SECTOR_SIZE);
    assert!(size_of::<SupplementaryVolumeDescriptor>() == SECTOR_SIZE);
    assert!(size_of::<BootRecordVolumeDescriptor>() == SECTOR_SIZE);
    assert!(size_of::<VolumeDescriptorSetTerminator>() == SECTOR_SIZE);
};

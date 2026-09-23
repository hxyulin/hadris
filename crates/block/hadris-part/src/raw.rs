//! On-disk MBR, EBR and GPT layouts.
//!
//! These types mirror the specification byte for byte: multi-byte fields are
//! little-endian byte arrays with decoding accessors of the same name, so
//! every layout is `Pod`, has alignment 1 and no padding. They may gain
//! items; existing items follow the specification.

use bytemuck::{Pod, Zeroable};

use crate::Guid;

/// The boot signature at bytes 510 and 511 of an MBR or EBR.
pub const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The signature of a GPT header, `"EFI PART"`.
pub const GPT_SIGNATURE: [u8; 8] = *b"EFI PART";

/// GPT header revision 1.0.
pub const GPT_REVISION: u32 = 0x0001_0000;

/// Size of the GPT header fields this crate reads and writes.
pub const GPT_HEADER_SIZE: u32 = 92;

/// Size of a GPT partition entry as this crate writes it.
pub const GPT_ENTRY_SIZE: u32 = 128;

/// Number of GPT entry slots this crate creates.
pub const GPT_DEFAULT_ENTRIES: u32 = 128;

/// MBR boot indicator of an active partition.
pub const MBR_ACTIVE: u8 = 0x80;

/// GPT attribute bit 0: required by the platform.
pub const GPT_ATTR_REQUIRED: u64 = 1 << 0;
/// GPT attribute bit 1: firmware must not produce a block I/O protocol for it.
pub const GPT_ATTR_NO_BLOCK_IO: u64 = 1 << 1;
/// GPT attribute bit 2: legacy BIOS bootable.
pub const GPT_ATTR_LEGACY_BIOS_BOOTABLE: u64 = 1 << 2;

/// A cylinder-head-sector address as stored in an MBR entry.
///
/// Addresses beyond 1023 cylinders of 255 heads and 63 sectors are stored as
/// `FF FF FF`, as the UEFI protective MBR requires.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Pod, Zeroable)]
pub struct Chs([u8; 3]);

impl Chs {
    const SECTORS: u32 = 63;
    const HEADS: u32 = 255;

    /// The address stored for blocks beyond CHS range.
    pub const OUT_OF_RANGE: Self = Self([0xFF, 0xFF, 0xFF]);

    /// Wraps the three on-disk bytes.
    pub const fn from_bytes(bytes: [u8; 3]) -> Self {
        Self(bytes)
    }

    /// The three on-disk bytes.
    pub const fn to_bytes(self) -> [u8; 3] {
        self.0
    }

    /// The address of `lba` on a 255-head, 63-sector geometry.
    pub const fn from_lba(lba: u32) -> Self {
        let cylinder = lba / (Self::SECTORS * Self::HEADS);
        if cylinder > 1023 {
            return Self::OUT_OF_RANGE;
        }
        let rest = lba % (Self::SECTORS * Self::HEADS);
        let head = rest / Self::SECTORS;
        let sector = rest % Self::SECTORS + 1;
        Self([
            head as u8,
            (sector as u8) | ((cylinder >> 2) as u8 & 0xC0),
            cylinder as u8,
        ])
    }

    /// The head, 0 to 255.
    pub const fn head(self) -> u8 {
        self.0[0]
    }

    /// The sector, 1 to 63; 0 is invalid.
    pub const fn sector(self) -> u8 {
        self.0[1] & 0x3F
    }

    /// The cylinder, 0 to 1023.
    pub const fn cylinder(self) -> u16 {
        ((self.0[1] as u16 & 0xC0) << 2) | self.0[2] as u16
    }

    /// The block this address names on a 255-head, 63-sector geometry, or
    /// `None` for sector 0, which the encoding does not allow.
    pub const fn to_lba(self) -> Option<u32> {
        if self.sector() == 0 {
            return None;
        }
        Some(
            self.cylinder() as u32 * Self::SECTORS * Self::HEADS
                + self.head() as u32 * Self::SECTORS
                + self.sector() as u32
                - 1,
        )
    }
}

/// One 16-byte partition entry of an MBR or EBR.
///
/// @hadris-spec MBR:partition-entry
/// @hadris-compliance unknown
/// @hadris-tests roundtrip::mbr_layout_roundtrip
/// @hadris-fuzz part_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Pod, Zeroable)]
pub struct RawMbrEntry {
    /// `0x80` for the active partition, otherwise `0x00`.
    pub boot_indicator: u8,
    /// Address of the first block.
    pub start_chs: Chs,
    /// Partition type code.
    pub kind: u8,
    /// Address of the last block.
    pub end_chs: Chs,
    /// First block, relative to the start of the disk (MBR) or as described
    /// for EBRs.
    pub start_lba: [u8; 4],
    /// Number of blocks.
    pub sector_count: [u8; 4],
}

impl RawMbrEntry {
    /// An entry for `sector_count` blocks from `start_lba`, with CHS
    /// addresses computed from them.
    pub const fn new(kind: u8, start_lba: u32, sector_count: u32) -> Self {
        let last = if sector_count == 0 {
            start_lba
        } else {
            start_lba.saturating_add(sector_count - 1)
        };
        Self {
            boot_indicator: 0,
            start_chs: Chs::from_lba(start_lba),
            kind,
            end_chs: Chs::from_lba(last),
            start_lba: start_lba.to_le_bytes(),
            sector_count: sector_count.to_le_bytes(),
        }
    }

    /// Decoded [`start_lba`](Self::start_lba) field.
    pub const fn start_lba(&self) -> u32 {
        u32::from_le_bytes(self.start_lba)
    }

    /// Decoded [`sector_count`](Self::sector_count) field.
    pub const fn sector_count(&self) -> u32 {
        u32::from_le_bytes(self.sector_count)
    }

    /// Whether the entry is unused (type code 0).
    pub const fn is_empty(&self) -> bool {
        self.kind == 0
    }
}

/// A 512-byte master boot record or extended boot record.
///
/// @hadris-spec MBR:layout
/// @hadris-compliance unknown
/// @hadris-tests roundtrip::mbr_layout_roundtrip
/// @hadris-fuzz part_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct RawMbr {
    /// Boot code.
    pub bootstrap: [u8; 440],
    /// Optional disk signature.
    pub disk_signature: [u8; 4],
    /// Usually zero; `5A 5A` marks a copy-protected disk.
    pub reserved: [u8; 2],
    /// The four partition entries.
    pub entries: [RawMbrEntry; 4],
    /// [`MBR_SIGNATURE`] on a valid record.
    pub signature: [u8; 2],
}

impl Default for RawMbr {
    fn default() -> Self {
        Self {
            signature: MBR_SIGNATURE,
            ..Zeroable::zeroed()
        }
    }
}

impl RawMbr {
    /// Whether bytes 510 and 511 hold [`MBR_SIGNATURE`].
    pub const fn has_signature(&self) -> bool {
        self.signature[0] == MBR_SIGNATURE[0] && self.signature[1] == MBR_SIGNATURE[1]
    }
}

/// The first 92 bytes of a GPT header block.
///
/// The CRC covers `header_size` bytes of the block, which may extend past
/// these fields.
///
/// @hadris-spec UEFI:GPT-Header
/// @hadris-compliance unknown
/// @hadris-tests read::backup_gpt_replaces_a_corrupt_primary, roundtrip::gpt_layout_roundtrip
/// @hadris-fuzz part_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct RawGptHeader {
    /// [`GPT_SIGNATURE`].
    pub signature: [u8; 8],
    /// Header revision.
    pub revision: [u8; 4],
    /// Bytes of the block covered by the header CRC.
    pub header_size: [u8; 4],
    /// CRC32 of the header with this field zero.
    pub header_crc32: [u8; 4],
    /// Must be zero.
    pub reserved: [u8; 4],
    /// Block holding this header.
    pub my_lba: [u8; 8],
    /// Block holding the other header.
    pub alternate_lba: [u8; 8],
    /// First block partitions may use.
    pub first_usable_lba: [u8; 8],
    /// Last block partitions may use.
    pub last_usable_lba: [u8; 8],
    /// Disk GUID, mixed-endian.
    pub disk_guid: [u8; 16],
    /// First block of this copy's partition entry array.
    pub partition_entry_lba: [u8; 8],
    /// Number of entries in the array.
    pub num_partition_entries: [u8; 4],
    /// Size of one entry, 128 times a power of two.
    pub size_of_partition_entry: [u8; 4],
    /// CRC32 of the entry array.
    pub partition_entry_array_crc32: [u8; 4],
}

impl RawGptHeader {
    /// Decoded [`revision`](Self::revision).
    pub const fn revision(&self) -> u32 {
        u32::from_le_bytes(self.revision)
    }

    /// Decoded [`header_size`](Self::header_size).
    pub const fn header_size(&self) -> u32 {
        u32::from_le_bytes(self.header_size)
    }

    /// Decoded [`header_crc32`](Self::header_crc32).
    pub const fn header_crc32(&self) -> u32 {
        u32::from_le_bytes(self.header_crc32)
    }

    /// Decoded [`my_lba`](Self::my_lba).
    pub const fn my_lba(&self) -> u64 {
        u64::from_le_bytes(self.my_lba)
    }

    /// Decoded [`alternate_lba`](Self::alternate_lba).
    pub const fn alternate_lba(&self) -> u64 {
        u64::from_le_bytes(self.alternate_lba)
    }

    /// Decoded [`first_usable_lba`](Self::first_usable_lba).
    pub const fn first_usable_lba(&self) -> u64 {
        u64::from_le_bytes(self.first_usable_lba)
    }

    /// Decoded [`last_usable_lba`](Self::last_usable_lba).
    pub const fn last_usable_lba(&self) -> u64 {
        u64::from_le_bytes(self.last_usable_lba)
    }

    /// Decoded [`disk_guid`](Self::disk_guid).
    pub const fn disk_guid(&self) -> Guid {
        Guid::from_bytes(self.disk_guid)
    }

    /// Decoded [`partition_entry_lba`](Self::partition_entry_lba).
    pub const fn partition_entry_lba(&self) -> u64 {
        u64::from_le_bytes(self.partition_entry_lba)
    }

    /// Decoded [`num_partition_entries`](Self::num_partition_entries).
    pub const fn num_partition_entries(&self) -> u32 {
        u32::from_le_bytes(self.num_partition_entries)
    }

    /// Decoded [`size_of_partition_entry`](Self::size_of_partition_entry).
    pub const fn size_of_partition_entry(&self) -> u32 {
        u32::from_le_bytes(self.size_of_partition_entry)
    }

    /// Decoded [`partition_entry_array_crc32`](Self::partition_entry_array_crc32).
    pub const fn partition_entry_array_crc32(&self) -> u32 {
        u32::from_le_bytes(self.partition_entry_array_crc32)
    }
}

/// The first 128 bytes of a GPT partition entry.
///
/// @hadris-spec UEFI:GPT-Entry
/// @hadris-compliance unknown
/// @hadris-tests roundtrip::gpt_layout_roundtrip, roundtrip::utf16_names_roundtrip
/// @hadris-fuzz part_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct RawGptEntry {
    /// Partition type GUID, mixed-endian; all zero for an unused entry.
    pub type_guid: [u8; 16],
    /// Unique partition GUID, mixed-endian.
    pub unique_guid: [u8; 16],
    /// First block.
    pub first_lba: [u8; 8],
    /// Last block, inclusive.
    pub last_lba: [u8; 8],
    /// Attribute bits.
    pub attributes: [u8; 8],
    /// Name, 36 UTF-16LE code units padded with zeros.
    pub name: [u8; 72],
}

impl Default for RawGptEntry {
    fn default() -> Self {
        Zeroable::zeroed()
    }
}

impl RawGptEntry {
    /// Decoded [`type_guid`](Self::type_guid).
    pub const fn type_guid(&self) -> Guid {
        Guid::from_bytes(self.type_guid)
    }

    /// Decoded [`unique_guid`](Self::unique_guid).
    pub const fn unique_guid(&self) -> Guid {
        Guid::from_bytes(self.unique_guid)
    }

    /// Decoded [`first_lba`](Self::first_lba).
    pub const fn first_lba(&self) -> u64 {
        u64::from_le_bytes(self.first_lba)
    }

    /// Decoded [`last_lba`](Self::last_lba).
    pub const fn last_lba(&self) -> u64 {
        u64::from_le_bytes(self.last_lba)
    }

    /// Decoded [`attributes`](Self::attributes).
    pub const fn attributes(&self) -> u64 {
        u64::from_le_bytes(self.attributes)
    }

    /// Whether the entry is unused (all-zero type GUID).
    pub const fn is_unused(&self) -> bool {
        let mut i = 0;
        while i < 16 {
            if self.type_guid[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Number of blocks from `first_lba` to `last_lba`, 0 when inverted and
    /// `u64::MAX` when the range covers every block.
    pub const fn block_len(&self) -> u64 {
        let first = self.first_lba();
        let last = self.last_lba();
        if last < first {
            0
        } else {
            (last - first).saturating_add(1)
        }
    }
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// Incremental CRC32 (ISO-HDLC, the GPT checksum).
#[derive(Debug, Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// A checksum over no bytes.
    pub const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    /// Adds `bytes`.
    pub fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.0;
        for &byte in bytes {
            crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.0 = crc;
    }

    /// The checksum of every byte added.
    pub const fn finish(self) -> u32 {
        !self.0
    }
}

/// CRC32 (ISO-HDLC) of `bytes`, as GPT headers store it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finish()
}

/// The CRC of a header block's first `header_size` bytes with the CRC
/// field treated as zero, or `None` when `header_size` is below 92 or past
/// the block.
pub fn gpt_header_crc(block: &[u8], header_size: u32) -> Option<u32> {
    let len = usize::try_from(header_size).ok()?;
    if len < GPT_HEADER_SIZE as usize || len > block.len() {
        return None;
    }
    let mut crc = Crc32::new();
    crc.update(&block[..16]);
    crc.update(&[0; 4]);
    crc.update(&block[20..len]);
    Some(crc.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_have_their_on_disk_sizes() {
        assert_eq!(size_of::<RawMbrEntry>(), 16);
        assert_eq!(size_of::<RawMbr>(), 512);
        assert_eq!(size_of::<RawGptHeader>(), 92);
        assert_eq!(size_of::<RawGptEntry>(), 128);
    }

    #[test]
    fn crc32_matches_the_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn chs_round_trips_within_range() {
        assert_eq!(Chs::from_lba(0).to_bytes(), [0, 1, 0]);
        assert_eq!(Chs::from_lba(1).to_bytes(), [0, 2, 0]);
        assert_eq!(Chs::from_lba(63).to_bytes(), [1, 1, 0]);
        assert_eq!(Chs::from_lba(63 * 255).to_bytes(), [0, 1, 1]);
        for lba in [0, 1, 62, 63, 16_064, 63 * 255 * 1023, 63 * 255 * 1024 - 1] {
            assert_eq!(Chs::from_lba(lba).to_lba(), Some(lba));
        }
        assert_eq!(Chs::from_lba(63 * 255 * 1024), Chs::OUT_OF_RANGE);
        assert_eq!(Chs::from_bytes([0, 0, 0]).to_lba(), None);
    }
}

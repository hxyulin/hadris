//! GPT (GUID Partition Table) types.
//!
//! This module provides types for working with GPT partition tables, including:
//! - GUID (Globally Unique Identifier)
//! - GPT partition table header
//! - GPT partition entries
//! - Well-known partition type GUIDs

use core::fmt::{Debug, Display};

use endian_num::Le;

/// A 128-bit GUID (Globally Unique Identifier).
///
/// GUIDs are stored in mixed-endian format:
/// - First 3 components (time_low, time_mid, time_hi_and_version) are little-endian
/// - Last 2 components (clock_seq, node) are big-endian
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Guid([u8; 16]);

impl Default for Guid {
    fn default() -> Self {
        Self::UNUSED
    }
}

impl Debug for Guid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Guid({self})")
    }
}

impl Display for Guid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let d1 = u32::from_le_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]);
        let d2 = u16::from_le_bytes([self.0[4], self.0[5]]);
        let d3 = u16::from_le_bytes([self.0[6], self.0[7]]);
        let d4 = &self.0[8..10];
        let d5 = &self.0[10..16];

        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            d1, d2, d3, d4[0], d4[1], d5[0], d5[1], d5[2], d5[3], d5[4], d5[5]
        )
    }
}

impl Guid {
    /// Creates a GUID from its raw bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes of this GUID.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0
    }

    /// Returns whether this GUID is all zeros (unused).
    pub const fn is_unused(&self) -> bool {
        let mut i = 0;
        while i < 16 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Creates a GUID from the standard string format.
    ///
    /// The format is: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
    /// where `x` is a hexadecimal digit.
    ///
    /// Returns `None` if the string is not in the correct format.
    pub const fn from_str(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() != 36 {
            return None;
        }

        // Check dashes at correct positions
        if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
            return None;
        }

        // Parse each component
        let d1 = match parse_hex_u32(&[
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]) {
            Some(v) => v,
            None => return None,
        };
        let d2 = match parse_hex_u16(&[bytes[9], bytes[10], bytes[11], bytes[12]]) {
            Some(v) => v,
            None => return None,
        };
        let d3 = match parse_hex_u16(&[bytes[14], bytes[15], bytes[16], bytes[17]]) {
            Some(v) => v,
            None => return None,
        };
        let d4_0 = match parse_hex_u8(&[bytes[19], bytes[20]]) {
            Some(v) => v,
            None => return None,
        };
        let d4_1 = match parse_hex_u8(&[bytes[21], bytes[22]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_0 = match parse_hex_u8(&[bytes[24], bytes[25]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_1 = match parse_hex_u8(&[bytes[26], bytes[27]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_2 = match parse_hex_u8(&[bytes[28], bytes[29]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_3 = match parse_hex_u8(&[bytes[30], bytes[31]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_4 = match parse_hex_u8(&[bytes[32], bytes[33]]) {
            Some(v) => v,
            None => return None,
        };
        let d5_5 = match parse_hex_u8(&[bytes[34], bytes[35]]) {
            Some(v) => v,
            None => return None,
        };

        // Convert to mixed-endian format
        let d1_bytes = d1.to_le_bytes();
        let d2_bytes = d2.to_le_bytes();
        let d3_bytes = d3.to_le_bytes();

        Some(Self([
            d1_bytes[0],
            d1_bytes[1],
            d1_bytes[2],
            d1_bytes[3],
            d2_bytes[0],
            d2_bytes[1],
            d3_bytes[0],
            d3_bytes[1],
            d4_0,
            d4_1,
            d5_0,
            d5_1,
            d5_2,
            d5_3,
            d5_4,
            d5_5,
        ]))
    }

    const fn from_canonical(s: &str) -> Self {
        match Self::from_str(s) {
            Some(g) => g,
            None => panic!("invalid GUID literal"),
        }
    }
}

// Helper functions for const GUID parsing
const fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

const fn parse_hex_u8(chars: &[u8; 2]) -> Option<u8> {
    let h = match hex_digit(chars[0]) {
        Some(v) => v,
        None => return None,
    };
    let l = match hex_digit(chars[1]) {
        Some(v) => v,
        None => return None,
    };
    Some((h << 4) | l)
}

const fn parse_hex_u16(chars: &[u8; 4]) -> Option<u16> {
    let b0 = match parse_hex_u8(&[chars[0], chars[1]]) {
        Some(v) => v,
        None => return None,
    };
    let b1 = match parse_hex_u8(&[chars[2], chars[3]]) {
        Some(v) => v,
        None => return None,
    };
    Some(((b0 as u16) << 8) | (b1 as u16))
}

const fn parse_hex_u32(chars: &[u8; 8]) -> Option<u32> {
    let b0 = match parse_hex_u16(&[chars[0], chars[1], chars[2], chars[3]]) {
        Some(v) => v,
        None => return None,
    };
    let b1 = match parse_hex_u16(&[chars[4], chars[5], chars[6], chars[7]]) {
        Some(v) => v,
        None => return None,
    };
    Some(((b0 as u32) << 16) | (b1 as u32))
}

// Well-known partition type GUIDs.
//
// Canonical values sourced from, and re-checkable against:
// - UEFI Specification 2.10, section 5.3 (EFI System Partition, unused entry)
// - Microsoft's documented partition type GUIDs (Microsoft section)
// - systemd/UAPI Discoverable Partitions Specification (Linux root/home/srv/swap):
//   https://uapi-group.org/specifications/specs/discoverable_partitions_specification/
// - Wikipedia "GUID Partition Table" partition type table, the aggregate
//   reference against which every value below was cross-checked on 2026-08-24:
//   https://en.wikipedia.org/wiki/GUID_Partition_Table#Partition_type_GUIDs
impl Guid {
    /// Unused/empty partition entry.
    pub const UNUSED: Self = Self([0; 16]);

    // === EFI/UEFI ===

    /// EFI System Partition (ESP).
    pub const EFI_SYSTEM: Self = Self::from_canonical("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");

    /// BIOS Boot Partition (for GRUB on GPT disks).
    pub const BIOS_BOOT: Self = Self::from_canonical("21686148-6449-6E6F-744E-656564454649");

    // === Microsoft ===

    /// Microsoft Reserved Partition (MSR).
    pub const MICROSOFT_RESERVED: Self =
        Self::from_canonical("E3C9E316-0B5C-4DB8-817D-F92DF00215AE");

    /// Basic Data Partition (Windows NTFS/FAT).
    pub const BASIC_DATA: Self = Self::from_canonical("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7");

    /// Windows LDM Metadata Partition.
    pub const WINDOWS_LDM_METADATA: Self =
        Self::from_canonical("5808C8AA-7E8F-42E0-85D2-E1E90434CFB3");

    /// Windows LDM Data Partition.
    pub const WINDOWS_LDM_DATA: Self = Self::from_canonical("AF9B60A0-1431-4F62-BC68-3311714A69AD");

    /// Windows Recovery Environment.
    pub const WINDOWS_RECOVERY: Self = Self::from_canonical("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC");

    /// Windows Storage Spaces.
    pub const WINDOWS_STORAGE_SPACES: Self =
        Self::from_canonical("E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D");

    // === Linux ===

    /// Linux Filesystem Data.
    pub const LINUX_FILESYSTEM: Self = Self::from_canonical("0FC63DAF-8483-4772-8E79-3D69D8477DE4");

    /// Linux RAID.
    pub const LINUX_RAID: Self = Self::from_canonical("A19D880F-05FC-4D3B-A006-743F0F84911E");

    /// Linux Root Partition (x86).
    pub const LINUX_ROOT_X86: Self = Self::from_canonical("44479540-F297-41B2-9AF7-D131D5F0458A");

    /// Linux Root Partition (x86-64).
    pub const LINUX_ROOT_X86_64: Self =
        Self::from_canonical("4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709");

    /// Linux Root Partition (ARM).
    pub const LINUX_ROOT_ARM: Self = Self::from_canonical("69DAD710-2CE4-4E3C-B16C-21A1D49ABED3");

    /// Linux Root Partition (ARM64/AArch64).
    pub const LINUX_ROOT_ARM64: Self = Self::from_canonical("B921B045-1DF0-41C3-AF44-4C6F280D3FAE");

    /// Linux Swap.
    pub const LINUX_SWAP: Self = Self::from_canonical("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F");

    /// Linux LVM.
    pub const LINUX_LVM: Self = Self::from_canonical("E6D6D379-F507-44C2-A23C-238F2A3DF928");

    /// Linux /home Partition.
    pub const LINUX_HOME: Self = Self::from_canonical("933AC7E1-2EB4-4F13-B844-0E14E2AEF915");

    /// Linux /srv (Server Data) Partition.
    pub const LINUX_SRV: Self = Self::from_canonical("3B8F8425-20E0-4F3B-907F-1A25A76F98E8");

    /// Linux dm-crypt / LUKS Partition.
    pub const LINUX_LUKS: Self = Self::from_canonical("CA7D7CCB-63ED-4C53-861C-1742536059CC");

    // === Apple ===

    /// Apple HFS+ Partition.
    pub const APPLE_HFS_PLUS: Self = Self::from_canonical("48465300-0000-11AA-AA11-00306543ECAC");

    /// Apple APFS Container.
    pub const APPLE_APFS: Self = Self::from_canonical("7C3457EF-0000-11AA-AA11-00306543ECAC");

    /// Apple UFS.
    pub const APPLE_UFS: Self = Self::from_canonical("55465300-0000-11AA-AA11-00306543ECAC");

    /// Apple RAID Partition.
    pub const APPLE_RAID: Self = Self::from_canonical("52414944-0000-11AA-AA11-00306543ECAC");

    /// Apple RAID Partition (offline).
    pub const APPLE_RAID_OFFLINE: Self =
        Self::from_canonical("52414944-5F4F-11AA-AA11-00306543ECAC");

    /// Apple Boot Partition (Recovery HD).
    pub const APPLE_BOOT: Self = Self::from_canonical("426F6F74-0000-11AA-AA11-00306543ECAC");

    /// Apple Label.
    pub const APPLE_LABEL: Self = Self::from_canonical("4C616265-6C00-11AA-AA11-00306543ECAC");

    /// Apple TV Recovery Partition.
    pub const APPLE_TV_RECOVERY: Self =
        Self::from_canonical("5265636F-7665-11AA-AA11-00306543ECAC");

    /// Apple Core Storage (FileVault).
    pub const APPLE_CORE_STORAGE: Self =
        Self::from_canonical("53746F72-6167-11AA-AA11-00306543ECAC");

    // === FreeBSD ===

    /// FreeBSD Boot Partition.
    pub const FREEBSD_BOOT: Self = Self::from_canonical("83BD6B9D-7F41-11DC-BE0B-001560B84F0F");

    /// FreeBSD Data Partition.
    pub const FREEBSD_DATA: Self = Self::from_canonical("516E7CB4-6ECF-11D6-8FF8-00022D09712B");

    /// FreeBSD Swap Partition.
    pub const FREEBSD_SWAP: Self = Self::from_canonical("516E7CB5-6ECF-11D6-8FF8-00022D09712B");

    /// FreeBSD UFS Partition.
    pub const FREEBSD_UFS: Self = Self::from_canonical("516E7CB6-6ECF-11D6-8FF8-00022D09712B");

    /// FreeBSD ZFS Partition.
    pub const FREEBSD_ZFS: Self = Self::from_canonical("516E7CBA-6ECF-11D6-8FF8-00022D09712B");

    /// FreeBSD Vinum/RAID Partition.
    pub const FREEBSD_VINUM: Self = Self::from_canonical("516E7CB8-6ECF-11D6-8FF8-00022D09712B");

    // === Solaris / illumos ===

    /// Solaris Boot Partition.
    pub const SOLARIS_BOOT: Self = Self::from_canonical("6A82CB45-1DD2-11B2-99A6-080020736631");

    /// Solaris Root Partition.
    pub const SOLARIS_ROOT: Self = Self::from_canonical("6A85CF4D-1DD2-11B2-99A6-080020736631");

    /// Solaris Swap Partition.
    pub const SOLARIS_SWAP: Self = Self::from_canonical("6A87C46F-1DD2-11B2-99A6-080020736631");

    /// Solaris Backup Partition.
    pub const SOLARIS_BACKUP: Self = Self::from_canonical("6A8B642B-1DD2-11B2-99A6-080020736631");

    /// Solaris /var Partition.
    pub const SOLARIS_VAR: Self = Self::from_canonical("6A8EF2E9-1DD2-11B2-99A6-080020736631");

    /// Solaris /home Partition.
    pub const SOLARIS_HOME: Self = Self::from_canonical("6A90BA39-1DD2-11B2-99A6-080020736631");

    /// Solaris Reserved.
    pub const SOLARIS_RESERVED: Self = Self::from_canonical("6A945A3B-1DD2-11B2-99A6-080020736631");

    // === NetBSD ===

    /// NetBSD Swap Partition.
    pub const NETBSD_SWAP: Self = Self::from_canonical("49F48D32-B10E-11DC-B99B-0019D1879648");

    /// NetBSD FFS Partition.
    pub const NETBSD_FFS: Self = Self::from_canonical("49F48D5A-B10E-11DC-B99B-0019D1879648");

    /// NetBSD LFS Partition.
    pub const NETBSD_LFS: Self = Self::from_canonical("49F48D82-B10E-11DC-B99B-0019D1879648");

    /// NetBSD RAID Partition.
    pub const NETBSD_RAID: Self = Self::from_canonical("49F48DAA-B10E-11DC-B99B-0019D1879648");

    // === Chrome OS ===

    /// Chrome OS Kernel.
    pub const CHROMEOS_KERNEL: Self = Self::from_canonical("FE3A2A5D-4F32-41A7-B725-ACCC3285A309");

    /// Chrome OS Root Filesystem.
    pub const CHROMEOS_ROOTFS: Self = Self::from_canonical("3CB8E202-3B7E-47DD-8A3C-7FF2A13CFCEC");

    /// Chrome OS Reserved (future use).
    pub const CHROMEOS_RESERVED: Self =
        Self::from_canonical("2E0A753D-9E48-43B0-8337-B15192CB1B5E");

    // === VMware ===

    /// VMware VMFS Partition.
    pub const VMWARE_VMFS: Self = Self::from_canonical("AA31E02A-400F-11DB-9590-000C2911D1B8");

    /// VMware Reserved.
    pub const VMWARE_RESERVED: Self = Self::from_canonical("9198EFFC-31C0-11DB-8F78-000C2911D1B8");
}

#[cfg(feature = "rand")]
impl Guid {
    /// Generate a new random GUID (version 4).
    pub fn generate_v4() -> Self {
        use rand::Rng;

        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);

        // Set version: 0100xxxx (version 4)
        bytes[7] = (bytes[7] & 0x0F) | 0x40;

        // Set variant: 10xxxxxx (RFC 4122)
        bytes[8] = (bytes[8] & 0x3F) | 0x80;

        Self(bytes)
    }
}

/// GPT partition entry attributes (64-bit flags, little-endian on disk).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GptAttributes(Le<u64>);

impl GptAttributes {
    /// Platform required (required for system to function).
    pub const PLATFORM_REQUIRED: u64 = 1 << 0;
    /// EFI should ignore this partition and not read from it.
    pub const EFI_IGNORE: u64 = 1 << 1;
    /// Legacy BIOS bootable (for MBR-style boot).
    pub const LEGACY_BIOS_BOOTABLE: u64 = 1 << 2;

    /// Creates new attributes from a raw value.
    pub const fn new(value: u64) -> Self {
        Self(Le::<u64>::from_ne(value))
    }

    /// Returns the raw attribute value.
    pub const fn get(&self) -> u64 {
        self.0.to_ne()
    }

    /// Sets the raw attribute value.
    pub fn set(&mut self, value: u64) {
        self.0 = Le::<u64>::from_ne(value);
    }

    /// Returns whether the platform required flag is set.
    pub const fn is_platform_required(&self) -> bool {
        (self.0.to_ne() & Self::PLATFORM_REQUIRED) != 0
    }

    /// Returns whether the EFI ignore flag is set.
    pub const fn is_efi_ignore(&self) -> bool {
        (self.0.to_ne() & Self::EFI_IGNORE) != 0
    }

    /// Returns whether the legacy BIOS bootable flag is set.
    pub const fn is_legacy_bios_bootable(&self) -> bool {
        (self.0.to_ne() & Self::LEGACY_BIOS_BOOTABLE) != 0
    }
}

impl Debug for GptAttributes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GptAttributes")
            .field("raw", &format_args!("0x{:016X}", self.get()))
            .field("platform_required", &self.is_platform_required())
            .field("efi_ignore", &self.is_efi_ignore())
            .field("legacy_bios_bootable", &self.is_legacy_bios_bootable())
            .finish()
    }
}

/// A UTF-16LE partition name (36 code units = 72 bytes).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GptPartitionName([u16; 36]);

// SAFETY: GptPartitionName is repr(C) containing only u16 values.
// All bit patterns are valid for u16.
unsafe impl bytemuck::Pod for GptPartitionName {}
unsafe impl bytemuck::Zeroable for GptPartitionName {}

impl Default for GptPartitionName {
    fn default() -> Self {
        Self([0; 36])
    }
}

impl Debug for GptPartitionName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Find null terminator
        let len = self.0.iter().position(|&c| c == 0).unwrap_or(36);
        // Simple ASCII display for debugging
        let mut s = [0u8; 36];
        for (i, &c) in self.0[..len].iter().enumerate() {
            s[i] = if c < 128 { c as u8 } else { b'?' };
        }
        write!(
            f,
            "GptPartitionName({:?})",
            core::str::from_utf8(&s[..len]).unwrap_or("?")
        )
    }
}

impl GptPartitionName {
    /// Creates a partition name from ASCII bytes.
    ///
    /// Non-ASCII characters and characters beyond 36 are ignored.
    pub const fn from_ascii(s: &[u8]) -> Self {
        let mut name = [0u16; 36];
        let len = if s.len() < 36 { s.len() } else { 36 };
        let mut i = 0;
        while i < len {
            if s[i] < 128 {
                name[i] = s[i] as u16;
            }
            i += 1;
        }
        Self(name)
    }

    /// Returns the raw UTF-16LE data.
    pub const fn as_u16_slice(&self) -> &[u16; 36] {
        &self.0
    }
}

/// GPT partition table header (92 bytes, padded to sector size).
///
/// Numeric fields use [`Le`] so their on-disk little-endian layout is explicit.
/// Use `to_raw` / `from_raw` or the I/O extension traits for serialization.
///
/// @hadris-spec UEFI:GPT-Header
/// @hadris-compliance unknown
/// @hadris-tests io_roundtrip::gpt_scheme_sync_write_open_and_detect_roundtrip
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GptHeader {
    /// Signature: must be "EFI PART" (0x5452415020494645).
    pub signature: [u8; 8],
    /// Revision: currently 0x00010000 (1.0), little-endian on disk.
    pub revision: Le<u32>,
    /// Header size in bytes (usually 92), little-endian on disk.
    pub header_size: Le<u32>,
    /// CRC32 of header (with this field set to 0 during calculation), little-endian on disk.
    pub header_crc32: Le<u32>,
    /// Reserved, must be 0, little-endian on disk.
    pub reserved: Le<u32>,
    /// LBA of this header, little-endian on disk.
    pub my_lba: Le<u64>,
    /// LBA of alternate header (backup), little-endian on disk.
    pub alternate_lba: Le<u64>,
    /// First usable LBA for partitions, little-endian on disk.
    pub first_usable_lba: Le<u64>,
    /// Last usable LBA for partitions, little-endian on disk.
    pub last_usable_lba: Le<u64>,
    /// Disk GUID.
    pub disk_guid: Guid,
    /// Starting LBA of partition entry array, little-endian on disk.
    pub partition_entry_lba: Le<u64>,
    /// Number of partition entries, little-endian on disk.
    pub num_partition_entries: Le<u32>,
    /// Size of each partition entry (usually 128), little-endian on disk.
    pub size_of_partition_entry: Le<u32>,
    /// CRC32 of partition entry array, little-endian on disk.
    pub partition_entry_array_crc32: Le<u32>,
}

impl Default for GptHeader {
    fn default() -> Self {
        Self {
            signature: *b"EFI PART",
            revision: Le::<u32>::from_ne(0x00010000),
            header_size: Le::<u32>::from_ne(92),
            header_crc32: Le::<u32>::from_ne(0),
            reserved: Le::<u32>::from_ne(0),
            my_lba: Le::<u64>::from_ne(0),
            alternate_lba: Le::<u64>::from_ne(0),
            first_usable_lba: Le::<u64>::from_ne(0),
            last_usable_lba: Le::<u64>::from_ne(0),
            disk_guid: Guid::default(),
            partition_entry_lba: Le::<u64>::from_ne(0),
            num_partition_entries: Le::<u32>::from_ne(0),
            size_of_partition_entry: Le::<u32>::from_ne(128),
            partition_entry_array_crc32: Le::<u32>::from_ne(0),
        }
    }
}

impl GptHeader {
    /// The required GPT signature.
    pub const SIGNATURE: [u8; 8] = *b"EFI PART";
    /// Current revision (1.0).
    pub const REVISION_1_0: u32 = 0x00010000;
    /// Standard header size.
    pub const STANDARD_HEADER_SIZE: u32 = 92;
    /// Standard partition entry size.
    pub const STANDARD_ENTRY_SIZE: u32 = 128;

    /// Returns whether the signature is valid.
    pub const fn has_valid_signature(&self) -> bool {
        self.signature[0] == b'E'
            && self.signature[1] == b'F'
            && self.signature[2] == b'I'
            && self.signature[3] == b' '
            && self.signature[4] == b'P'
            && self.signature[5] == b'A'
            && self.signature[6] == b'R'
            && self.signature[7] == b'T'
    }

    /// Calculates the CRC32 of this header.
    ///
    /// The header_crc32 field is treated as 0 during calculation.
    #[cfg(feature = "crc")]
    pub fn calculate_crc32(&self) -> u32 {
        use crc::{CRC_32_ISO_HDLC, Crc};
        const HASHER: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

        let mut header = *self;
        header.header_crc32 = Le::<u32>::from_ne(0);
        let raw = header.to_raw();
        HASHER.checksum(bytemuck::bytes_of(&raw))
    }

    /// Verifies the header CRC32.
    #[cfg(feature = "crc")]
    pub fn verify_crc32(&self) -> bool {
        self.header_crc32.to_ne() == self.calculate_crc32()
    }

    /// Updates the header CRC32 field.
    #[cfg(feature = "crc")]
    pub fn update_crc32(&mut self) {
        self.header_crc32 = Le::<u32>::from_ne(0);
        self.header_crc32 = Le::<u32>::from_ne(self.calculate_crc32());
    }
}

/// On-disk GPT header representation (92 bytes, packed).
///
/// This struct matches the exact on-disk layout of the GPT header.
/// The [`GptHeader`] struct uses native alignment and may include padding;
/// this packed representation is used for serialization and CRC.
#[repr(C, packed)]
#[derive(Clone, Copy)]
#[cfg(any(
    feature = "crc",
    all(
        any(feature = "read", feature = "write"),
        any(feature = "sync", feature = "async")
    )
))]
pub(crate) struct GptHeaderRaw {
    signature: [u8; 8],
    revision: Le<u32>,
    header_size: Le<u32>,
    header_crc32: Le<u32>,
    reserved: Le<u32>,
    my_lba: Le<u64>,
    alternate_lba: Le<u64>,
    first_usable_lba: Le<u64>,
    last_usable_lba: Le<u64>,
    disk_guid: [u8; 16],
    partition_entry_lba: Le<u64>,
    num_partition_entries: Le<u32>,
    size_of_partition_entry: Le<u32>,
    partition_entry_array_crc32: Le<u32>,
}

// SAFETY: GptHeaderRaw is repr(C, packed) with Pod field types (`Le`, byte arrays).
// All bit patterns are valid.
#[cfg(any(
    feature = "crc",
    all(
        any(feature = "read", feature = "write"),
        any(feature = "sync", feature = "async")
    )
))]
unsafe impl bytemuck::Pod for GptHeaderRaw {}
#[cfg(any(
    feature = "crc",
    all(
        any(feature = "read", feature = "write"),
        any(feature = "sync", feature = "async")
    )
))]
unsafe impl bytemuck::Zeroable for GptHeaderRaw {}

#[cfg(all(
    any(feature = "read", feature = "write"),
    any(feature = "sync", feature = "async")
))]
impl GptHeaderRaw {
    /// Size of the raw header on disk.
    pub(crate) const SIZE: usize = 92;
}

impl GptHeader {
    /// Converts this header to its on-disk packed representation.
    #[cfg(any(
        feature = "crc",
        all(feature = "write", any(feature = "sync", feature = "async"))
    ))]
    pub(crate) fn to_raw(self) -> GptHeaderRaw {
        GptHeaderRaw {
            signature: self.signature,
            revision: self.revision,
            header_size: self.header_size,
            header_crc32: self.header_crc32,
            reserved: self.reserved,
            my_lba: self.my_lba,
            alternate_lba: self.alternate_lba,
            first_usable_lba: self.first_usable_lba,
            last_usable_lba: self.last_usable_lba,
            disk_guid: self.disk_guid.to_bytes(),
            partition_entry_lba: self.partition_entry_lba,
            num_partition_entries: self.num_partition_entries,
            size_of_partition_entry: self.size_of_partition_entry,
            partition_entry_array_crc32: self.partition_entry_array_crc32,
        }
    }

    /// Creates a header from its on-disk packed representation.
    #[cfg(all(feature = "read", any(feature = "sync", feature = "async")))]
    pub(crate) fn from_raw(raw: &GptHeaderRaw) -> Self {
        Self {
            signature: raw.signature,
            revision: raw.revision,
            header_size: raw.header_size,
            header_crc32: raw.header_crc32,
            reserved: raw.reserved,
            my_lba: raw.my_lba,
            alternate_lba: raw.alternate_lba,
            first_usable_lba: raw.first_usable_lba,
            last_usable_lba: raw.last_usable_lba,
            disk_guid: Guid::from_bytes(raw.disk_guid),
            partition_entry_lba: raw.partition_entry_lba,
            num_partition_entries: raw.num_partition_entries,
            size_of_partition_entry: raw.size_of_partition_entry,
            partition_entry_array_crc32: raw.partition_entry_array_crc32,
        }
    }
}

/// GPT partition entry (128 bytes by default).
///
/// @hadris-spec UEFI:GPT-Entry
/// @hadris-compliance unknown
/// @hadris-tests roundtrip::gpt_partition_entry_roundtrip
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GptPartitionEntry {
    /// Partition type GUID.
    pub type_guid: Guid,
    /// Unique partition GUID.
    pub unique_guid: Guid,
    /// First LBA, little-endian on disk.
    pub first_lba: Le<u64>,
    /// Last LBA (inclusive), little-endian on disk.
    pub last_lba: Le<u64>,
    /// Attribute flags.
    pub attributes: GptAttributes,
    /// Partition name (UTF-16LE).
    pub name: GptPartitionName,
}

impl Default for GptPartitionEntry {
    fn default() -> Self {
        Self {
            type_guid: Guid::UNUSED,
            unique_guid: Guid::UNUSED,
            first_lba: Le::<u64>::from_ne(0),
            last_lba: Le::<u64>::from_ne(0),
            attributes: GptAttributes::default(),
            name: GptPartitionName::default(),
        }
    }
}

impl Debug for GptPartitionEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GptPartitionEntry")
            .field("type_guid", &self.type_guid)
            .field("unique_guid", &self.unique_guid)
            .field("first_lba", &self.first_lba.to_ne())
            .field("last_lba", &self.last_lba.to_ne())
            .field("attributes", &self.attributes)
            .field("name", &self.name)
            .finish()
    }
}

impl GptPartitionEntry {
    /// Creates a new partition entry.
    pub const fn new(type_guid: Guid, unique_guid: Guid, first_lba: u64, last_lba: u64) -> Self {
        Self {
            type_guid,
            unique_guid,
            first_lba: Le::<u64>::from_ne(first_lba),
            last_lba: Le::<u64>::from_ne(last_lba),
            attributes: GptAttributes::new(0),
            name: GptPartitionName([0; 36]),
        }
    }

    /// Returns whether this entry is unused (empty).
    pub const fn is_unused(&self) -> bool {
        self.type_guid.is_unused()
    }

    /// Returns the partition size in sectors.
    ///
    /// Saturates to `u64::MAX`: an entry spanning `first_lba = 0` to
    /// `last_lba = u64::MAX` is representable on disk but its size exceeds
    /// `u64`.
    pub const fn size_sectors(&self) -> u64 {
        let first = self.first_lba.to_ne();
        let last = self.last_lba.to_ne();
        if self.is_unused() || last < first {
            0
        } else {
            (last - first).saturating_add(1)
        }
    }

    /// Returns the partition size in bytes (assuming 512-byte sectors).
    pub const fn size_bytes(&self) -> u64 {
        self.size_sectors().saturating_mul(512)
    }

    /// Returns the partition size in bytes for a given sector size.
    pub const fn size_bytes_with_sector_size(&self, sector_size: u32) -> u64 {
        self.size_sectors().saturating_mul(sector_size as u64)
    }

    /// Sets the partition name from ASCII.
    pub fn set_name_ascii(&mut self, name: &[u8]) {
        self.name = GptPartitionName::from_ascii(name);
    }
}

/// Calculates the CRC32 of a partition entry array.
#[cfg(feature = "crc")]
pub fn calculate_partition_array_crc32(entries: &[GptPartitionEntry]) -> u32 {
    use crc::{CRC_32_ISO_HDLC, Crc};
    const HASHER: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);
    HASHER.checksum(bytemuck::cast_slice(entries))
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::PartitionInfoTrait;
    use alloc::format;

    #[test]
    fn test_guid_display() {
        let guid = Guid::EFI_SYSTEM;
        let s = format!("{guid}");
        assert_eq!(s, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
    }

    #[test]
    fn test_guid_from_str() {
        let guid = Guid::from_str("c12a7328-f81f-11d2-ba4b-00a0c93ec93b").unwrap();
        assert_eq!(guid, Guid::EFI_SYSTEM);
    }

    #[test]
    fn test_guid_is_unused() {
        assert!(Guid::UNUSED.is_unused());
        assert!(!Guid::EFI_SYSTEM.is_unused());
    }

    #[test]
    fn test_well_known_guids_match_canonical_strings() {
        const CANONICAL: &[(Guid, &str)] = &[
            (Guid::EFI_SYSTEM, "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
            (Guid::BIOS_BOOT, "21686148-6449-6E6F-744E-656564454649"),
            (
                Guid::MICROSOFT_RESERVED,
                "E3C9E316-0B5C-4DB8-817D-F92DF00215AE",
            ),
            (Guid::BASIC_DATA, "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"),
            (
                Guid::WINDOWS_LDM_METADATA,
                "5808C8AA-7E8F-42E0-85D2-E1E90434CFB3",
            ),
            (
                Guid::WINDOWS_LDM_DATA,
                "AF9B60A0-1431-4F62-BC68-3311714A69AD",
            ),
            (
                Guid::WINDOWS_RECOVERY,
                "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC",
            ),
            (
                Guid::WINDOWS_STORAGE_SPACES,
                "E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D",
            ),
            (
                Guid::LINUX_FILESYSTEM,
                "0FC63DAF-8483-4772-8E79-3D69D8477DE4",
            ),
            (Guid::LINUX_RAID, "A19D880F-05FC-4D3B-A006-743F0F84911E"),
            (Guid::LINUX_ROOT_X86, "44479540-F297-41B2-9AF7-D131D5F0458A"),
            (
                Guid::LINUX_ROOT_X86_64,
                "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
            ),
            (Guid::LINUX_ROOT_ARM, "69DAD710-2CE4-4E3C-B16C-21A1D49ABED3"),
            (
                Guid::LINUX_ROOT_ARM64,
                "B921B045-1DF0-41C3-AF44-4C6F280D3FAE",
            ),
            (Guid::LINUX_SWAP, "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F"),
            (Guid::LINUX_LVM, "E6D6D379-F507-44C2-A23C-238F2A3DF928"),
            (Guid::LINUX_HOME, "933AC7E1-2EB4-4F13-B844-0E14E2AEF915"),
            (Guid::LINUX_SRV, "3B8F8425-20E0-4F3B-907F-1A25A76F98E8"),
            (Guid::LINUX_LUKS, "CA7D7CCB-63ED-4C53-861C-1742536059CC"),
            (Guid::APPLE_HFS_PLUS, "48465300-0000-11AA-AA11-00306543ECAC"),
            (Guid::APPLE_APFS, "7C3457EF-0000-11AA-AA11-00306543ECAC"),
            (Guid::APPLE_UFS, "55465300-0000-11AA-AA11-00306543ECAC"),
            (Guid::APPLE_RAID, "52414944-0000-11AA-AA11-00306543ECAC"),
            (
                Guid::APPLE_RAID_OFFLINE,
                "52414944-5F4F-11AA-AA11-00306543ECAC",
            ),
            (Guid::APPLE_BOOT, "426F6F74-0000-11AA-AA11-00306543ECAC"),
            (Guid::APPLE_LABEL, "4C616265-6C00-11AA-AA11-00306543ECAC"),
            (
                Guid::APPLE_TV_RECOVERY,
                "5265636F-7665-11AA-AA11-00306543ECAC",
            ),
            (
                Guid::APPLE_CORE_STORAGE,
                "53746F72-6167-11AA-AA11-00306543ECAC",
            ),
            (Guid::FREEBSD_BOOT, "83BD6B9D-7F41-11DC-BE0B-001560B84F0F"),
            (Guid::FREEBSD_DATA, "516E7CB4-6ECF-11D6-8FF8-00022D09712B"),
            (Guid::FREEBSD_SWAP, "516E7CB5-6ECF-11D6-8FF8-00022D09712B"),
            (Guid::FREEBSD_UFS, "516E7CB6-6ECF-11D6-8FF8-00022D09712B"),
            (Guid::FREEBSD_ZFS, "516E7CBA-6ECF-11D6-8FF8-00022D09712B"),
            (Guid::FREEBSD_VINUM, "516E7CB8-6ECF-11D6-8FF8-00022D09712B"),
            (Guid::SOLARIS_BOOT, "6A82CB45-1DD2-11B2-99A6-080020736631"),
            (Guid::SOLARIS_ROOT, "6A85CF4D-1DD2-11B2-99A6-080020736631"),
            (Guid::SOLARIS_SWAP, "6A87C46F-1DD2-11B2-99A6-080020736631"),
            (Guid::SOLARIS_BACKUP, "6A8B642B-1DD2-11B2-99A6-080020736631"),
            (Guid::SOLARIS_VAR, "6A8EF2E9-1DD2-11B2-99A6-080020736631"),
            (Guid::SOLARIS_HOME, "6A90BA39-1DD2-11B2-99A6-080020736631"),
            (
                Guid::SOLARIS_RESERVED,
                "6A945A3B-1DD2-11B2-99A6-080020736631",
            ),
            (Guid::NETBSD_SWAP, "49F48D32-B10E-11DC-B99B-0019D1879648"),
            (Guid::NETBSD_FFS, "49F48D5A-B10E-11DC-B99B-0019D1879648"),
            (Guid::NETBSD_LFS, "49F48D82-B10E-11DC-B99B-0019D1879648"),
            (Guid::NETBSD_RAID, "49F48DAA-B10E-11DC-B99B-0019D1879648"),
            (
                Guid::CHROMEOS_KERNEL,
                "FE3A2A5D-4F32-41A7-B725-ACCC3285A309",
            ),
            (
                Guid::CHROMEOS_ROOTFS,
                "3CB8E202-3B7E-47DD-8A3C-7FF2A13CFCEC",
            ),
            (
                Guid::CHROMEOS_RESERVED,
                "2E0A753D-9E48-43B0-8337-B15192CB1B5E",
            ),
            (Guid::VMWARE_VMFS, "AA31E02A-400F-11DB-9590-000C2911D1B8"),
            (
                Guid::VMWARE_RESERVED,
                "9198EFFC-31C0-11DB-8F78-000C2911D1B8",
            ),
        ];

        assert_eq!(
            format!("{}", Guid::UNUSED),
            "00000000-0000-0000-0000-000000000000"
        );
        for (guid, canonical) in CANONICAL {
            assert_eq!(
                format!("{guid}"),
                canonical.to_ascii_lowercase(),
                "constant does not match canonical GUID {canonical}"
            );
        }
    }

    #[test]
    fn test_well_known_guids_roundtrip() {
        for guid in [
            Guid::UNUSED,
            Guid::EFI_SYSTEM,
            Guid::BIOS_BOOT,
            Guid::BASIC_DATA,
            Guid::LINUX_FILESYSTEM,
            Guid::APPLE_APFS,
            Guid::FREEBSD_ZFS,
            Guid::SOLARIS_ROOT,
            Guid::NETBSD_RAID,
            Guid::CHROMEOS_KERNEL,
            Guid::VMWARE_VMFS,
        ] {
            let s = format!("{guid}");
            assert_eq!(Guid::from_str(&s), Some(guid));
        }
    }

    #[test]
    fn test_gpt_header_size() {
        // GptHeader uses native alignment, so the size may be larger than 92 bytes
        // due to padding. The on-disk format is 92 bytes.
        assert!(core::mem::size_of::<GptHeader>() >= 92);
    }

    #[test]
    fn test_gpt_partition_entry_size() {
        assert_eq!(core::mem::size_of::<GptPartitionEntry>(), 128);
    }

    #[test]
    fn test_partition_entry_size_sectors() {
        let entry = GptPartitionEntry::new(Guid::LINUX_FILESYSTEM, Guid::UNUSED, 2048, 4095);
        assert_eq!(entry.size_sectors(), 2048);
        assert_eq!(entry.byte_len(512).unwrap(), 2048 * 512);
    }

    #[test]
    fn test_partition_name_ascii() {
        let name = GptPartitionName::from_ascii(b"EFI System");
        let slice = name.as_u16_slice();
        assert_eq!(slice[0], b'E' as u16);
        assert_eq!(slice[1], b'F' as u16);
        assert_eq!(slice[2], b'I' as u16);
        assert_eq!(slice[10], 0); // null after "EFI System"
    }
}

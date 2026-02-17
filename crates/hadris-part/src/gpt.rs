//! GPT (GUID Partition Table) types.
//!
//! This module provides types for working with GPT partition tables, including:
//! - GUID (Globally Unique Identifier)
//! - GPT partition table header
//! - GPT partition entries
//! - Well-known partition type GUIDs

use core::fmt::{Debug, Display};

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
        write!(f, "Guid({})", self)
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

// Well-known partition type GUIDs
impl Guid {
    /// Unused/empty partition entry.
    pub const UNUSED: Self = Self([0; 16]);

    // === EFI/UEFI ===

    /// EFI System Partition (ESP).
    pub const EFI_SYSTEM: Self = Self([
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);

    /// BIOS Boot Partition (for GRUB on GPT disks).
    pub const BIOS_BOOT: Self = Self([
        0x48, 0x61, 0x68, 0x21, 0x49, 0x64, 0x6f, 0x6e, 0x74, 0x4e, 0x65, 0x65, 0x64, 0x45, 0x46,
        0x49,
    ]);

    // === Microsoft ===

    /// Microsoft Reserved Partition (MSR).
    pub const MICROSOFT_RESERVED: Self = Self([
        0x16, 0xe3, 0xc9, 0xe3, 0x5c, 0x0b, 0xb8, 0x4d, 0x81, 0x7d, 0xf9, 0x2d, 0xf0, 0x02, 0x15,
        0xae,
    ]);

    /// Basic Data Partition (Windows NTFS/FAT).
    pub const BASIC_DATA: Self = Self([
        0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99,
        0xc7,
    ]);

    /// Windows LDM Metadata Partition.
    pub const WINDOWS_LDM_METADATA: Self = Self([
        0xaa, 0xc8, 0x08, 0x58, 0x8f, 0x7e, 0xe0, 0x42, 0x85, 0xd2, 0xe1, 0xe9, 0x04, 0x34, 0xcf,
        0xb3,
    ]);

    /// Windows LDM Data Partition.
    pub const WINDOWS_LDM_DATA: Self = Self([
        0xa0, 0x60, 0x9b, 0xaf, 0x31, 0x14, 0x62, 0x4f, 0xbc, 0x68, 0x33, 0x11, 0x71, 0x4a, 0x69,
        0xad,
    ]);

    /// Windows Recovery Environment.
    pub const WINDOWS_RECOVERY: Self = Self([
        0xa4, 0xbb, 0x94, 0xde, 0xd1, 0x06, 0x40, 0x4d, 0xa1, 0x6a, 0xbf, 0xd5, 0x01, 0x79, 0xd6,
        0xac,
    ]);

    /// Windows Storage Spaces.
    pub const WINDOWS_STORAGE_SPACES: Self = Self([
        0xe7, 0x5c, 0xaf, 0xe7, 0xa0, 0x1a, 0x4d, 0x4d, 0xbe, 0xe7, 0x47, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    // === Linux ===

    /// Linux Filesystem Data.
    pub const LINUX_FILESYSTEM: Self = Self([
        0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d,
        0xe4,
    ]);

    /// Linux RAID.
    pub const LINUX_RAID: Self = Self([
        0x0f, 0x88, 0x9d, 0xa1, 0xfc, 0x05, 0x3b, 0x4d, 0xa0, 0x06, 0x74, 0x3f, 0x0f, 0x84, 0x91,
        0x1e,
    ]);

    /// Linux Root Partition (x86).
    pub const LINUX_ROOT_X86: Self = Self([
        0x10, 0xd7, 0xda, 0x44, 0x96, 0xe5, 0xe9, 0x4c, 0xb1, 0x6b, 0x00, 0xa5, 0x24, 0x00, 0x00,
        0x00,
    ]);

    /// Linux Root Partition (x86-64).
    pub const LINUX_ROOT_X86_64: Self = Self([
        0x02, 0xb9, 0x2f, 0x4f, 0xbe, 0x39, 0x3e, 0x4e, 0xb2, 0xc8, 0x23, 0x54, 0x61, 0x6c, 0x02,
        0x72,
    ]);

    /// Linux Root Partition (ARM).
    pub const LINUX_ROOT_ARM: Self = Self([
        0xb1, 0x21, 0xb0, 0x69, 0xda, 0xf3, 0x4c, 0x4c, 0x8d, 0x5a, 0x52, 0x57, 0x4c, 0x02, 0x37,
        0x00,
    ]);

    /// Linux Root Partition (ARM64/AArch64).
    pub const LINUX_ROOT_ARM64: Self = Self([
        0xd5, 0x7e, 0x44, 0xb7, 0x85, 0x00, 0x4c, 0x49, 0x8a, 0x82, 0x7f, 0x05, 0x39, 0x45, 0x00,
        0x00,
    ]);

    /// Linux Swap.
    pub const LINUX_SWAP: Self = Self([
        0x6d, 0xfd, 0x57, 0x06, 0xab, 0xa4, 0xc4, 0x43, 0x84, 0xe5, 0x09, 0x33, 0xc8, 0x4b, 0x4f,
        0x4f,
    ]);

    /// Linux LVM.
    pub const LINUX_LVM: Self = Self([
        0x79, 0xd3, 0xd6, 0xe6, 0x07, 0xf5, 0xc2, 0x44, 0xa2, 0x3c, 0x23, 0x8f, 0x2a, 0x3d, 0xf9,
        0x28,
    ]);

    /// Linux /home Partition.
    pub const LINUX_HOME: Self = Self([
        0x33, 0xe7, 0xd1, 0x93, 0xd8, 0x0c, 0x4d, 0x4e, 0x90, 0x4a, 0xac, 0x2d, 0x04, 0x00, 0x00,
        0x00,
    ]);

    /// Linux /srv (Server Data) Partition.
    pub const LINUX_SRV: Self = Self([
        0x33, 0xe7, 0xd1, 0x93, 0xd8, 0x0c, 0x4d, 0x4e, 0x90, 0x4a, 0xac, 0x2d, 0x05, 0x00, 0x00,
        0x00,
    ]);

    /// Linux dm-crypt / LUKS Partition.
    pub const LINUX_LUKS: Self = Self([
        0x7f, 0xff, 0xff, 0xca, 0xbc, 0xcd, 0x43, 0x4d, 0xa9, 0x17, 0x87, 0xe1, 0x14, 0x00, 0x00,
        0x00,
    ]);

    // === Apple ===

    /// Apple HFS+ Partition.
    pub const APPLE_HFS_PLUS: Self = Self([
        0x00, 0x53, 0x46, 0x48, 0x00, 0x00, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple APFS Container.
    pub const APPLE_APFS: Self = Self([
        0xef, 0x57, 0x34, 0x7c, 0x00, 0x00, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple UFS.
    pub const APPLE_UFS: Self = Self([
        0x00, 0x53, 0x46, 0x55, 0x00, 0x00, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple RAID Partition.
    pub const APPLE_RAID: Self = Self([
        0x2d, 0x52, 0x41, 0x49, 0x00, 0x00, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple RAID Partition (offline).
    pub const APPLE_RAID_OFFLINE: Self = Self([
        0x2d, 0x52, 0x41, 0x49, 0x4f, 0x46, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple Boot Partition (Recovery HD).
    pub const APPLE_BOOT: Self = Self([
        0x00, 0x74, 0x6f, 0x6f, 0x42, 0x65, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple Label.
    pub const APPLE_LABEL: Self = Self([
        0x6c, 0x65, 0x62, 0x61, 0x4c, 0x00, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple TV Recovery Partition.
    pub const APPLE_TV_RECOVERY: Self = Self([
        0x52, 0x65, 0x63, 0x76, 0x65, 0x72, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    /// Apple Core Storage (FileVault).
    pub const APPLE_CORE_STORAGE: Self = Self([
        0xb6, 0x7c, 0x6e, 0x53, 0x56, 0x43, 0xaa, 0x11, 0xaa, 0x11, 0x00, 0x30, 0x65, 0x43, 0xec,
        0xac,
    ]);

    // === FreeBSD ===

    /// FreeBSD Boot Partition.
    pub const FREEBSD_BOOT: Self = Self([
        0x00, 0x08, 0x00, 0x83, 0x01, 0x00, 0xb0, 0x11, 0x00, 0x00, 0xe4, 0x00, 0x00, 0x69, 0x00,
        0x00,
    ]);

    /// FreeBSD Data Partition.
    pub const FREEBSD_DATA: Self = Self([
        0xa5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// FreeBSD Swap Partition.
    pub const FREEBSD_SWAP: Self = Self([
        0x00, 0x08, 0x00, 0x83, 0x01, 0x00, 0xb0, 0x11, 0x00, 0x01, 0xe4, 0x00, 0x00, 0x69, 0x00,
        0x00,
    ]);

    /// FreeBSD UFS Partition.
    pub const FREEBSD_UFS: Self = Self([
        0x00, 0x08, 0x00, 0x83, 0x01, 0x00, 0xb0, 0x11, 0x00, 0x02, 0xe4, 0x00, 0x00, 0x69, 0x00,
        0x00,
    ]);

    /// FreeBSD ZFS Partition.
    pub const FREEBSD_ZFS: Self = Self([
        0x00, 0x08, 0x00, 0x83, 0x01, 0x00, 0xb0, 0x11, 0x00, 0x05, 0xe4, 0x00, 0x00, 0x69, 0x00,
        0x00,
    ]);

    /// FreeBSD Vinum/RAID Partition.
    pub const FREEBSD_VINUM: Self = Self([
        0x00, 0x08, 0x00, 0x83, 0x01, 0x00, 0xb0, 0x11, 0x00, 0x03, 0xe4, 0x00, 0x00, 0x69, 0x00,
        0x00,
    ]);

    // === Solaris / illumos ===

    /// Solaris Boot Partition.
    pub const SOLARIS_BOOT: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris Root Partition.
    pub const SOLARIS_ROOT: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris Swap Partition.
    pub const SOLARIS_SWAP: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris Backup Partition.
    pub const SOLARIS_BACKUP: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris /var Partition.
    pub const SOLARIS_VAR: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x56, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris /home Partition.
    pub const SOLARIS_HOME: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x57, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Solaris Reserved.
    pub const SOLARIS_RESERVED: Self = Self([
        0x45, 0xcb, 0x82, 0x6a, 0xd2, 0x1d, 0x11, 0xd3, 0x81, 0x5f, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    // === NetBSD ===

    /// NetBSD Swap Partition.
    pub const NETBSD_SWAP: Self = Self([
        0x32, 0x8d, 0xf4, 0x49, 0x19, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);

    /// NetBSD FFS Partition.
    pub const NETBSD_FFS: Self = Self([
        0x32, 0x8d, 0xf4, 0x49, 0x19, 0xf8, 0xd2, 0x11, 0xba, 0x4c, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);

    /// NetBSD LFS Partition.
    pub const NETBSD_LFS: Self = Self([
        0x32, 0x8d, 0xf4, 0x49, 0x19, 0xf8, 0xd2, 0x11, 0xba, 0x4d, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);

    /// NetBSD RAID Partition.
    pub const NETBSD_RAID: Self = Self([
        0x32, 0x8d, 0xf4, 0x49, 0x19, 0xf8, 0xd2, 0x11, 0xba, 0x4f, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);

    // === Chrome OS ===

    /// Chrome OS Kernel.
    pub const CHROMEOS_KERNEL: Self = Self([
        0x5d, 0x2a, 0x3a, 0xfe, 0x32, 0x4f, 0xa7, 0x41, 0xb7, 0x25, 0xac, 0xcc, 0x32, 0x85, 0xa3,
        0x09,
    ]);

    /// Chrome OS Root Filesystem.
    pub const CHROMEOS_ROOTFS: Self = Self([
        0x02, 0xe2, 0xb8, 0x3c, 0x7e, 0x3b, 0xdd, 0x47, 0x8a, 0x3c, 0x7f, 0xf2, 0xa1, 0x3c, 0xfc,
        0xec,
    ]);

    /// Chrome OS Reserved (future use).
    pub const CHROMEOS_RESERVED: Self = Self([
        0x3d, 0x75, 0x0a, 0x2e, 0x48, 0x9e, 0xb0, 0x43, 0x83, 0x37, 0xb1, 0x51, 0x92, 0xcb, 0x1b,
        0x5e,
    ]);

    // === VMware ===

    /// VMware VMFS Partition.
    pub const VMWARE_VMFS: Self = Self([
        0x10, 0x04, 0x1d, 0xaa, 0xd1, 0xf4, 0x50, 0x4a, 0x98, 0xba, 0xfa, 0x28, 0x80, 0x9d, 0x61,
        0x12,
    ]);

    /// VMware Reserved.
    pub const VMWARE_RESERVED: Self = Self([
        0x8d, 0x61, 0x00, 0x99, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);
}

#[cfg(feature = "rand")]
impl Guid {
    /// Generate a new random GUID (version 4).
    pub fn generate_v4() -> Self {
        use rand::RngCore;

        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);

        // Set version: 0100xxxx (version 4)
        bytes[6] = (bytes[6] & 0x0F) | 0x40;

        // Set variant: 10xxxxxx (RFC 4122)
        bytes[8] = (bytes[8] & 0x3F) | 0x80;

        Self(bytes)
    }
}

/// GPT partition entry attributes.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GptAttributes(u64);

impl GptAttributes {
    /// Platform required (required for system to function).
    pub const PLATFORM_REQUIRED: u64 = 1 << 0;
    /// EFI should ignore this partition and not read from it.
    pub const EFI_IGNORE: u64 = 1 << 1;
    /// Legacy BIOS bootable (for MBR-style boot).
    pub const LEGACY_BIOS_BOOTABLE: u64 = 1 << 2;

    /// Creates new attributes from a raw value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw attribute value.
    pub const fn get(&self) -> u64 {
        self.0
    }

    /// Sets the raw attribute value.
    pub fn set(&mut self, value: u64) {
        self.0 = value;
    }

    /// Returns whether the platform required flag is set.
    pub const fn is_platform_required(&self) -> bool {
        (self.0 & Self::PLATFORM_REQUIRED) != 0
    }

    /// Returns whether the EFI ignore flag is set.
    pub const fn is_efi_ignore(&self) -> bool {
        (self.0 & Self::EFI_IGNORE) != 0
    }

    /// Returns whether the legacy BIOS bootable flag is set.
    pub const fn is_legacy_bios_bootable(&self) -> bool {
        (self.0 & Self::LEGACY_BIOS_BOOTABLE) != 0
    }
}

impl Debug for GptAttributes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GptAttributes")
            .field("raw", &format_args!("0x{:016X}", self.0))
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
/// Note: This struct uses native alignment for ease of use. When reading/writing
/// to disk, use the serialization methods rather than direct memory casting.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GptHeader {
    /// Signature: must be "EFI PART" (0x5452415020494645).
    pub signature: [u8; 8],
    /// Revision: currently 0x00010000 (1.0).
    pub revision: u32,
    /// Header size in bytes (usually 92).
    pub header_size: u32,
    /// CRC32 of header (with this field set to 0 during calculation).
    pub header_crc32: u32,
    /// Reserved, must be 0.
    pub reserved: u32,
    /// LBA of this header.
    pub my_lba: u64,
    /// LBA of alternate header (backup).
    pub alternate_lba: u64,
    /// First usable LBA for partitions.
    pub first_usable_lba: u64,
    /// Last usable LBA for partitions.
    pub last_usable_lba: u64,
    /// Disk GUID.
    pub disk_guid: Guid,
    /// Starting LBA of partition entry array.
    pub partition_entry_lba: u64,
    /// Number of partition entries.
    pub num_partition_entries: u32,
    /// Size of each partition entry (usually 128).
    pub size_of_partition_entry: u32,
    /// CRC32 of partition entry array.
    pub partition_entry_array_crc32: u32,
}

impl Default for GptHeader {
    fn default() -> Self {
        Self {
            signature: *b"EFI PART",
            revision: 0x00010000,
            header_size: 92,
            header_crc32: 0,
            reserved: 0,
            my_lba: 0,
            alternate_lba: 0,
            first_usable_lba: 0,
            last_usable_lba: 0,
            disk_guid: Guid::default(),
            partition_entry_lba: 0,
            num_partition_entries: 0,
            size_of_partition_entry: 128,
            partition_entry_array_crc32: 0,
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
        header.header_crc32 = 0;
        let raw = header.to_raw();
        HASHER.checksum(bytemuck::bytes_of(&raw))
    }

    /// Verifies the header CRC32.
    #[cfg(feature = "crc")]
    pub fn verify_crc32(&self) -> bool {
        self.header_crc32 == self.calculate_crc32()
    }

    /// Updates the header CRC32 field.
    #[cfg(feature = "crc")]
    pub fn update_crc32(&mut self) {
        self.header_crc32 = 0;
        self.header_crc32 = self.calculate_crc32();
    }
}

/// On-disk GPT header representation (92 bytes, packed).
///
/// This struct matches the exact on-disk layout of the GPT header.
/// The `GptHeader` struct uses native alignment for convenience but may
/// have padding, so this packed representation is used for serialization.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GptHeaderRaw {
    signature: [u8; 8],
    revision: [u8; 4],
    header_size: [u8; 4],
    header_crc32: [u8; 4],
    reserved: [u8; 4],
    my_lba: [u8; 8],
    alternate_lba: [u8; 8],
    first_usable_lba: [u8; 8],
    last_usable_lba: [u8; 8],
    disk_guid: [u8; 16],
    partition_entry_lba: [u8; 8],
    num_partition_entries: [u8; 4],
    size_of_partition_entry: [u8; 4],
    partition_entry_array_crc32: [u8; 4],
}

// SAFETY: GptHeaderRaw is repr(C, packed) with only byte arrays.
// All bit patterns are valid.
unsafe impl bytemuck::Pod for GptHeaderRaw {}
unsafe impl bytemuck::Zeroable for GptHeaderRaw {}

impl GptHeaderRaw {
    /// Size of the raw header on disk.
    const SIZE: usize = 92;
}

impl GptHeader {
    /// Converts this header to its on-disk packed representation.
    fn to_raw(self) -> GptHeaderRaw {
        GptHeaderRaw {
            signature: self.signature,
            revision: self.revision.to_le_bytes(),
            header_size: self.header_size.to_le_bytes(),
            header_crc32: self.header_crc32.to_le_bytes(),
            reserved: self.reserved.to_le_bytes(),
            my_lba: self.my_lba.to_le_bytes(),
            alternate_lba: self.alternate_lba.to_le_bytes(),
            first_usable_lba: self.first_usable_lba.to_le_bytes(),
            last_usable_lba: self.last_usable_lba.to_le_bytes(),
            disk_guid: self.disk_guid.to_bytes(),
            partition_entry_lba: self.partition_entry_lba.to_le_bytes(),
            num_partition_entries: self.num_partition_entries.to_le_bytes(),
            size_of_partition_entry: self.size_of_partition_entry.to_le_bytes(),
            partition_entry_array_crc32: self.partition_entry_array_crc32.to_le_bytes(),
        }
    }

    /// Creates a header from its on-disk packed representation.
    fn from_raw(raw: &GptHeaderRaw) -> Self {
        Self {
            signature: raw.signature,
            revision: u32::from_le_bytes(raw.revision),
            header_size: u32::from_le_bytes(raw.header_size),
            header_crc32: u32::from_le_bytes(raw.header_crc32),
            reserved: u32::from_le_bytes(raw.reserved),
            my_lba: u64::from_le_bytes(raw.my_lba),
            alternate_lba: u64::from_le_bytes(raw.alternate_lba),
            first_usable_lba: u64::from_le_bytes(raw.first_usable_lba),
            last_usable_lba: u64::from_le_bytes(raw.last_usable_lba),
            disk_guid: Guid::from_bytes(raw.disk_guid),
            partition_entry_lba: u64::from_le_bytes(raw.partition_entry_lba),
            num_partition_entries: u32::from_le_bytes(raw.num_partition_entries),
            size_of_partition_entry: u32::from_le_bytes(raw.size_of_partition_entry),
            partition_entry_array_crc32: u32::from_le_bytes(raw.partition_entry_array_crc32),
        }
    }
}

// I/O operations for GptHeader
#[cfg(feature = "read")]
impl GptHeader {
    /// Reads a GPT header from a reader.
    ///
    /// The reader should be positioned at the start of the header (typically LBA 1).
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the signature is invalid.
    pub fn read_from<R: hadris_io::Read>(reader: &mut R) -> crate::error::Result<Self> {
        let mut buf = [0u8; GptHeaderRaw::SIZE];
        reader
            .read_exact(&mut buf)
            .map_err(|_| crate::error::PartitionError::Io)?;
        let raw: GptHeaderRaw = bytemuck::cast(buf);
        let header = Self::from_raw(&raw);

        if !header.has_valid_signature() {
            return Err(crate::error::PartitionError::InvalidGptSignature {
                found: header.signature,
            });
        }

        Ok(header)
    }

    /// Reads a GPT header from a specific LBA.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking/reading fails or if the signature is invalid.
    pub fn read_from_lba<R: hadris_io::Read + hadris_io::Seek>(
        reader: &mut R,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<Self> {
        reader
            .seek(hadris_io::SeekFrom::Start(lba * block_size as u64))
            .map_err(|_| crate::error::PartitionError::Io)?;
        Self::read_from(reader)
    }
}

#[cfg(feature = "write")]
impl GptHeader {
    /// Writes this GPT header to a writer.
    ///
    /// Only writes the 92-byte header, not padding to sector size.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to<W: hadris_io::Write>(&self, writer: &mut W) -> crate::error::Result<()> {
        let raw = self.to_raw();
        writer
            .write_all(bytemuck::bytes_of(&raw))
            .map_err(|_| crate::error::PartitionError::Io)
    }

    /// Writes this GPT header to a specific LBA, padded to block size.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking/writing fails.
    pub fn write_to_lba<W: hadris_io::Write + hadris_io::Seek>(
        &self,
        writer: &mut W,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<()> {
        writer
            .seek(hadris_io::SeekFrom::Start(lba * block_size as u64))
            .map_err(|_| crate::error::PartitionError::Io)?;

        let raw = self.to_raw();
        writer
            .write_all(bytemuck::bytes_of(&raw))
            .map_err(|_| crate::error::PartitionError::Io)?;

        // Pad to block size
        let padding_size = block_size as usize - GptHeaderRaw::SIZE;
        if padding_size > 0 {
            let padding = [0u8; 512]; // Use 512 as max typical block size
            writer
                .write_all(&padding[..padding_size.min(512)])
                .map_err(|_| crate::error::PartitionError::Io)?;
        }

        Ok(())
    }
}

/// GPT partition entry (128 bytes by default).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GptPartitionEntry {
    /// Partition type GUID.
    pub type_guid: Guid,
    /// Unique partition GUID.
    pub unique_guid: Guid,
    /// First LBA (little-endian).
    pub first_lba: u64,
    /// Last LBA (inclusive, little-endian).
    pub last_lba: u64,
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
            first_lba: 0,
            last_lba: 0,
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
            .field("first_lba", &self.first_lba)
            .field("last_lba", &self.last_lba)
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
            first_lba,
            last_lba,
            attributes: GptAttributes::new(0),
            name: GptPartitionName([0; 36]),
        }
    }

    /// Returns whether this entry is unused (empty).
    pub const fn is_unused(&self) -> bool {
        self.type_guid.is_unused()
    }

    /// Returns the partition size in sectors.
    pub const fn size_sectors(&self) -> u64 {
        if self.is_unused() || self.last_lba < self.first_lba {
            0
        } else {
            self.last_lba - self.first_lba + 1
        }
    }

    /// Returns the partition size in bytes (assuming 512-byte sectors).
    pub const fn size_bytes(&self) -> u64 {
        self.size_sectors() * 512
    }

    /// Returns the partition size in bytes for a given sector size.
    pub const fn size_bytes_with_sector_size(&self, sector_size: u32) -> u64 {
        self.size_sectors() * sector_size as u64
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
    use alloc::format;

    #[test]
    fn test_guid_display() {
        let guid = Guid::EFI_SYSTEM;
        let s = format!("{}", guid);
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
        assert_eq!(entry.size_bytes(), 2048 * 512);
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

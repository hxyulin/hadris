//! Well-known GPT partition type GUIDs.
//!
//! Values come from the UEFI specification (EFI system, unused), Microsoft's
//! documented types, the UAPI Discoverable Partitions Specification (Linux
//! root, home, srv, swap), and the partition type table of the Wikipedia
//! article "GUID Partition Table", against which each was checked.

use crate::Guid;

const fn guid(text: &str) -> Guid {
    match Guid::parse_const(text) {
        Some(guid) => guid,
        None => panic!("invalid GUID literal"),
    }
}

/// An unused entry.
pub const UNUSED: Guid = Guid::NIL;

/// EFI system partition.
pub const EFI_SYSTEM: Guid = guid("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
/// BIOS boot partition (GRUB on GPT).
pub const BIOS_BOOT: Guid = guid("21686148-6449-6E6F-744E-656564454649");

/// Microsoft reserved partition.
pub const MICROSOFT_RESERVED: Guid = guid("E3C9E316-0B5C-4DB8-817D-F92DF00215AE");
/// Microsoft basic data (FAT, exFAT, NTFS).
pub const BASIC_DATA: Guid = guid("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7");
/// Windows LDM metadata.
pub const WINDOWS_LDM_METADATA: Guid = guid("5808C8AA-7E8F-42E0-85D2-E1E90434CFB3");
/// Windows LDM data.
pub const WINDOWS_LDM_DATA: Guid = guid("AF9B60A0-1431-4F62-BC68-3311714A69AD");
/// Windows recovery environment.
pub const WINDOWS_RECOVERY: Guid = guid("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC");
/// Windows Storage Spaces.
pub const WINDOWS_STORAGE_SPACES: Guid = guid("E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D");

/// Linux filesystem data.
pub const LINUX_FILESYSTEM: Guid = guid("0FC63DAF-8483-4772-8E79-3D69D8477DE4");
/// Linux RAID.
pub const LINUX_RAID: Guid = guid("A19D880F-05FC-4D3B-A006-743F0F84911E");
/// Linux root (x86).
pub const LINUX_ROOT_X86: Guid = guid("44479540-F297-41B2-9AF7-D131D5F0458A");
/// Linux root (x86-64).
pub const LINUX_ROOT_X86_64: Guid = guid("4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709");
/// Linux root (32-bit ARM).
pub const LINUX_ROOT_ARM: Guid = guid("69DAD710-2CE4-4E3C-B16C-21A1D49ABED3");
/// Linux root (AArch64).
pub const LINUX_ROOT_ARM64: Guid = guid("B921B045-1DF0-41C3-AF44-4C6F280D3FAE");
/// Linux swap.
pub const LINUX_SWAP: Guid = guid("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F");
/// Linux LVM.
pub const LINUX_LVM: Guid = guid("E6D6D379-F507-44C2-A23C-238F2A3DF928");
/// Linux `/home`.
pub const LINUX_HOME: Guid = guid("933AC7E1-2EB4-4F13-B844-0E14E2AEF915");
/// Linux `/srv`.
pub const LINUX_SRV: Guid = guid("3B8F8425-20E0-4F3B-907F-1A25A76F98E8");
/// Linux dm-crypt or LUKS.
pub const LINUX_LUKS: Guid = guid("CA7D7CCB-63ED-4C53-861C-1742536059CC");

/// Apple HFS+.
pub const APPLE_HFS_PLUS: Guid = guid("48465300-0000-11AA-AA11-00306543ECAC");
/// Apple APFS container.
pub const APPLE_APFS: Guid = guid("7C3457EF-0000-11AA-AA11-00306543ECAC");
/// Apple UFS.
pub const APPLE_UFS: Guid = guid("55465300-0000-11AA-AA11-00306543ECAC");
/// Apple RAID.
pub const APPLE_RAID: Guid = guid("52414944-0000-11AA-AA11-00306543ECAC");
/// Apple RAID, offline.
pub const APPLE_RAID_OFFLINE: Guid = guid("52414944-5F4F-11AA-AA11-00306543ECAC");
/// Apple boot (Recovery HD).
pub const APPLE_BOOT: Guid = guid("426F6F74-0000-11AA-AA11-00306543ECAC");
/// Apple label.
pub const APPLE_LABEL: Guid = guid("4C616265-6C00-11AA-AA11-00306543ECAC");
/// Apple TV recovery.
pub const APPLE_TV_RECOVERY: Guid = guid("5265636F-7665-11AA-AA11-00306543ECAC");
/// Apple Core Storage.
pub const APPLE_CORE_STORAGE: Guid = guid("53746F72-6167-11AA-AA11-00306543ECAC");

/// FreeBSD boot.
pub const FREEBSD_BOOT: Guid = guid("83BD6B9D-7F41-11DC-BE0B-001560B84F0F");
/// FreeBSD data.
pub const FREEBSD_DATA: Guid = guid("516E7CB4-6ECF-11D6-8FF8-00022D09712B");
/// FreeBSD swap.
pub const FREEBSD_SWAP: Guid = guid("516E7CB5-6ECF-11D6-8FF8-00022D09712B");
/// FreeBSD UFS.
pub const FREEBSD_UFS: Guid = guid("516E7CB6-6ECF-11D6-8FF8-00022D09712B");
/// FreeBSD ZFS.
pub const FREEBSD_ZFS: Guid = guid("516E7CBA-6ECF-11D6-8FF8-00022D09712B");
/// FreeBSD Vinum.
pub const FREEBSD_VINUM: Guid = guid("516E7CB8-6ECF-11D6-8FF8-00022D09712B");

/// Solaris boot.
pub const SOLARIS_BOOT: Guid = guid("6A82CB45-1DD2-11B2-99A6-080020736631");
/// Solaris root.
pub const SOLARIS_ROOT: Guid = guid("6A85CF4D-1DD2-11B2-99A6-080020736631");
/// Solaris swap.
pub const SOLARIS_SWAP: Guid = guid("6A87C46F-1DD2-11B2-99A6-080020736631");
/// Solaris backup.
pub const SOLARIS_BACKUP: Guid = guid("6A8B642B-1DD2-11B2-99A6-080020736631");
/// Solaris `/var`.
pub const SOLARIS_VAR: Guid = guid("6A8EF2E9-1DD2-11B2-99A6-080020736631");
/// Solaris `/home`.
pub const SOLARIS_HOME: Guid = guid("6A90BA39-1DD2-11B2-99A6-080020736631");
/// Solaris reserved.
pub const SOLARIS_RESERVED: Guid = guid("6A945A3B-1DD2-11B2-99A6-080020736631");

/// NetBSD swap.
pub const NETBSD_SWAP: Guid = guid("49F48D32-B10E-11DC-B99B-0019D1879648");
/// NetBSD FFS.
pub const NETBSD_FFS: Guid = guid("49F48D5A-B10E-11DC-B99B-0019D1879648");
/// NetBSD LFS.
pub const NETBSD_LFS: Guid = guid("49F48D82-B10E-11DC-B99B-0019D1879648");
/// NetBSD RAID.
pub const NETBSD_RAID: Guid = guid("49F48DAA-B10E-11DC-B99B-0019D1879648");

/// ChromeOS kernel.
pub const CHROMEOS_KERNEL: Guid = guid("FE3A2A5D-4F32-41A7-B725-ACCC3285A309");
/// ChromeOS root filesystem.
pub const CHROMEOS_ROOTFS: Guid = guid("3CB8E202-3B7E-47DD-8A3C-7FF2A13CFCEC");
/// ChromeOS reserved.
pub const CHROMEOS_RESERVED: Guid = guid("2E0A753D-9E48-43B0-8337-B15192CB1B5E");

/// VMware VMFS.
pub const VMWARE_VMFS: Guid = guid("AA31E02A-400F-11DB-9590-000C2911D1B8");
/// VMware reserved.
pub const VMWARE_RESERVED: Guid = guid("9198EFFC-31C0-11DB-8F78-000C2911D1B8");

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::string::ToString;

    #[test]
    fn constants_match_their_canonical_text() {
        let canonical: &[(Guid, &str)] = &[
            (UNUSED, "00000000-0000-0000-0000-000000000000"),
            (EFI_SYSTEM, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"),
            (BIOS_BOOT, "21686148-6449-6e6f-744e-656564454649"),
            (MICROSOFT_RESERVED, "e3c9e316-0b5c-4db8-817d-f92df00215ae"),
            (BASIC_DATA, "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7"),
            (LINUX_FILESYSTEM, "0fc63daf-8483-4772-8e79-3d69d8477de4"),
            (LINUX_ROOT_X86_64, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709"),
            (LINUX_SWAP, "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f"),
            (APPLE_APFS, "7c3457ef-0000-11aa-aa11-00306543ecac"),
            (FREEBSD_ZFS, "516e7cba-6ecf-11d6-8ff8-00022d09712b"),
            (SOLARIS_ROOT, "6a85cf4d-1dd2-11b2-99a6-080020736631"),
            (NETBSD_RAID, "49f48daa-b10e-11dc-b99b-0019d1879648"),
            (CHROMEOS_KERNEL, "fe3a2a5d-4f32-41a7-b725-accc3285a309"),
            (VMWARE_RESERVED, "9198effc-31c0-11db-8f78-000c2911d1b8"),
        ];
        for (guid, text) in canonical {
            assert_eq!(guid.to_string(), *text);
            assert_eq!(text.parse::<Guid>().unwrap(), *guid);
        }
    }
}

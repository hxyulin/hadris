//! BIOS parameter block validation and volume geometry.

use hadris_common::types::endian::Endian;

use super::entry::{
    FAT12_MAX_CLUSTERS, FAT16_MAX_CLUSTERS, FAT32_MAX_CLUSTERS, FIRST_DATA_CLUSTER, FatKind,
};
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};

pub(crate) const FSINFO_LEAD_SIG: u32 = 0x4161_5252;
pub(crate) const FSINFO_STRUC_SIG: u32 = 0x6141_7272;
pub(crate) const FSINFO_TRAIL_SIG: u32 = 0xAA55_0000;
pub(crate) const BOOT_SIGNATURE: u16 = 0xAA55;

/// Why a boot sector or FSInfo sector was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootError {
    /// A field violates the specification; the text names it.
    Corrupt(&'static str),
    /// The boot sector does not end in `0xAA55`.
    Signature(u16),
    /// The FAT32 root cluster is not a data cluster.
    RootCluster { cluster: u32, max: u32 },
    /// An FSInfo signature is wrong.
    FsInfoSignature {
        field: &'static str,
        expected: u32,
        found: u32,
    },
}

/// Where the root directory lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootDir {
    /// FAT12/16: a fixed region of `size` bytes at byte `start`.
    Fixed { start: u64, size: u64 },
    /// FAT32: an ordinary cluster chain.
    Cluster(u32),
}

/// Byte layout of a FAT volume, derived from a validated boot sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub(crate) kind: FatKind,
    pub(crate) sector_size: u32,
    pub(crate) cluster_size: u32,
    pub(crate) fat_start: u64,
    /// Bytes in one FAT copy.
    pub(crate) fat_size: u64,
    pub(crate) fat_count: u8,
    pub(crate) root: RootDir,
    pub(crate) data_start: u64,
    /// The highest valid cluster number; clusters start at 2.
    pub(crate) max_cluster: u32,
}

impl Geometry {
    /// Byte offset of `cluster`'s first byte, or `None` when `cluster` is
    /// not a data cluster.
    pub(crate) const fn cluster_offset(&self, cluster: u32) -> Option<u64> {
        if cluster < FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return None;
        }
        Some(self.data_start + (cluster - FIRST_DATA_CLUSTER) as u64 * self.cluster_size as u64)
    }
}

/// Checks the fields common to every FAT variant: sector size, sectors per
/// cluster and cluster size.
pub(crate) fn check_bpb(bpb: &RawBpb) -> Result<(), BootError> {
    let sector_size = bpb.bytes_per_sector.get() as u32;
    if !matches!(sector_size, 512 | 1024 | 2048 | 4096) {
        return Err(BootError::Corrupt(
            "BPB bytes_per_sector must be 512, 1024, 2048, or 4096",
        ));
    }
    if !bpb.sectors_per_cluster.is_power_of_two() || bpb.sectors_per_cluster > 128 {
        return Err(BootError::Corrupt(
            "BPB sectors_per_cluster must be a power of two from 1 through 128",
        ));
    }
    if bpb.sectors_per_cluster as u32 * sector_size > 32 * 1024 {
        return Err(BootError::Corrupt(
            "BPB cluster size must not exceed 32 KiB",
        ));
    }
    Ok(())
}

/// Whether the BPB describes a FAT32 layout: no fixed root directory and no
/// 16-bit FAT size.
pub(crate) fn is_fat32(bpb: &RawBpb) -> bool {
    bpb.root_entry_count == [0, 0] && bpb.sectors_per_fat_16 == [0, 0]
}

fn check_fat_count(bpb: &RawBpb) -> Result<(), BootError> {
    if bpb.fat_count != 1 && bpb.fat_count != 2 {
        return Err(BootError::Corrupt("BPB fat_count must be 1 or 2"));
    }
    Ok(())
}

/// Checks the FAT12/16 extended boot record.
pub(crate) fn check_ext16(bpb: &RawBpb, ext: &RawBpbExt16) -> Result<(), BootError> {
    let signature = u16::from_le_bytes(ext.signature_word);
    if signature != BOOT_SIGNATURE {
        return Err(BootError::Signature(signature));
    }
    check_fat_count(bpb)
}

/// Checks the FAT32 extended boot record.
pub(crate) fn check_ext32(bpb: &RawBpb, ext: &RawBpbExt32) -> Result<(), BootError> {
    let signature = ext.signature_word.get();
    if signature != BOOT_SIGNATURE {
        return Err(BootError::Signature(signature));
    }
    if ext.version != [0, 0] {
        return Err(BootError::Corrupt("unsupported FAT32 filesystem version"));
    }
    check_fat_count(bpb)
}

/// Checks the three FSInfo signatures.
pub(crate) fn check_fs_info(info: &RawFsInfo) -> Result<(), BootError> {
    let signatures = [
        (
            "FSI_LeadSig",
            FSINFO_LEAD_SIG,
            u32::from_le_bytes(info.signature),
        ),
        (
            "FSI_StrucSig",
            FSINFO_STRUC_SIG,
            u32::from_le_bytes(info.structure_signature),
        ),
        ("FSI_TrailSig", FSINFO_TRAIL_SIG, info.trail_signature.get()),
    ];
    for (field, expected, found) in signatures {
        if found != expected {
            return Err(BootError::FsInfoSignature {
                field,
                expected,
                found,
            });
        }
    }
    Ok(())
}

fn total_sectors(bpb: &RawBpb) -> u64 {
    if bpb.total_sectors_16 != [0, 0] {
        u16::from_le_bytes(bpb.total_sectors_16) as u64
    } else {
        u32::from_le_bytes(bpb.total_sectors_32) as u64
    }
}

fn clusters(bpb: &RawBpb, metadata_sectors: u64) -> Result<u64, BootError> {
    total_sectors(bpb)
        .saturating_sub(metadata_sectors)
        .checked_div(bpb.sectors_per_cluster as u64)
        .ok_or(BootError::Corrupt(
            "BPB sectors_per_cluster must not be zero",
        ))
}

/// The geometry of a FAT12/16 volume. FAT12 when it has fewer than 4085
/// clusters. A layout with more clusters than FAT16 addresses, or without a
/// root directory, is rejected, since by count it would be FAT32. Expects
/// [`check_bpb`] and [`check_ext16`] to have passed.
pub(crate) fn geometry16(bpb: &RawBpb) -> Result<Geometry, BootError> {
    let sector_size = bpb.bytes_per_sector.get() as u64;
    let reserved = bpb.reserved_sector_count.get() as u64;
    let fat_sectors = u16::from_le_bytes(bpb.sectors_per_fat_16) as u64;
    let root_size = u16::from_le_bytes(bpb.root_entry_count) as u64 * 32;
    if root_size == 0 {
        return Err(BootError::Corrupt(
            "BPB root_entry_count must not be zero on FAT12/16",
        ));
    }
    let root_sectors = root_size.div_ceil(sector_size);
    let fat_start = reserved * sector_size;
    let root_start = fat_start + bpb.fat_count as u64 * fat_sectors * sector_size;
    let count = clusters(
        bpb,
        reserved + bpb.fat_count as u64 * fat_sectors + root_sectors,
    )?;
    if count > FAT16_MAX_CLUSTERS as u64 {
        return Err(BootError::Corrupt(
            "FAT12/16 layout has more clusters than FAT16 can address",
        ));
    }
    Ok(Geometry {
        kind: if count <= FAT12_MAX_CLUSTERS as u64 {
            FatKind::Fat12
        } else {
            FatKind::Fat16
        },
        sector_size: sector_size as u32,
        cluster_size: bpb.sectors_per_cluster as u32 * sector_size as u32,
        fat_start,
        fat_size: fat_sectors * sector_size,
        fat_count: bpb.fat_count,
        root: RootDir::Fixed {
            start: root_start,
            size: root_size,
        },
        data_start: root_start + root_sectors * sector_size,
        max_cluster: count as u32 + 1,
    })
}

/// The geometry of a FAT32 volume. A layout with more clusters than FAT32
/// addresses is rejected. One with fewer than 65525 clusters, which by
/// count would be FAT16, is read as FAT32, as Linux and macOS do: `mkfs.fat
/// -F 32` makes such volumes. Expects [`check_bpb`] and [`check_ext32`] to
/// have passed.
pub(crate) fn geometry32(bpb: &RawBpb, ext: &RawBpbExt32) -> Result<Geometry, BootError> {
    let sector_size = bpb.bytes_per_sector.get() as u64;
    let reserved = bpb.reserved_sector_count.get() as u64;
    let fat_sectors = ext.sectors_per_fat_32.get() as u64;
    let fat_start = reserved * sector_size;
    let count = clusters(bpb, reserved + fat_sectors * bpb.fat_count as u64)?;
    if count > FAT32_MAX_CLUSTERS as u64 {
        return Err(BootError::Corrupt(
            "FAT32 layout has more clusters than FAT32 can address",
        ));
    }
    let max_cluster = count as u32 + 1;
    let root = ext.root_cluster.get();
    if !(FIRST_DATA_CLUSTER..=max_cluster).contains(&root) {
        return Err(BootError::RootCluster {
            cluster: root,
            max: max_cluster,
        });
    }
    Ok(Geometry {
        kind: FatKind::Fat32,
        sector_size: sector_size as u32,
        cluster_size: bpb.sectors_per_cluster as u32 * sector_size as u32,
        fat_start,
        fat_size: fat_sectors * sector_size,
        fat_count: bpb.fat_count,
        root: RootDir::Cluster(root),
        data_start: fat_start + bpb.fat_count as u64 * fat_sectors * sector_size,
        max_cluster,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bpb(
        sector_size: u16,
        spc: u8,
        reserved: u16,
        fats: u8,
        root_entries: u16,
        spf16: u16,
        total: u32,
    ) -> RawBpb {
        let mut bpb: RawBpb = bytemuck::Zeroable::zeroed();
        bpb.bytes_per_sector.set(sector_size);
        bpb.sectors_per_cluster = spc;
        bpb.reserved_sector_count.set(reserved);
        bpb.fat_count = fats;
        bpb.root_entry_count = root_entries.to_le_bytes();
        bpb.sectors_per_fat_16 = spf16.to_le_bytes();
        if total <= u16::MAX as u32 {
            bpb.total_sectors_16 = (total as u16).to_le_bytes();
        } else {
            bpb.total_sectors_32 = total.to_le_bytes();
        }
        bpb
    }

    #[test]
    fn check_bpb_rejects_bad_sizes() {
        assert_eq!(check_bpb(&bpb(512, 1, 1, 2, 224, 9, 2880)), Ok(()));
        assert!(matches!(
            check_bpb(&bpb(500, 1, 1, 2, 224, 9, 2880)),
            Err(BootError::Corrupt(_))
        ));
        assert!(matches!(
            check_bpb(&bpb(512, 3, 1, 2, 224, 9, 2880)),
            Err(BootError::Corrupt(_))
        ));
        assert!(matches!(
            check_bpb(&bpb(512, 0, 1, 2, 224, 9, 2880)),
            Err(BootError::Corrupt(_))
        ));
        assert!(matches!(
            check_bpb(&bpb(4096, 16, 1, 2, 224, 9, 2880)),
            Err(BootError::Corrupt(_))
        ));
    }

    #[test]
    fn floppy_is_fat12() {
        let bpb = bpb(512, 1, 1, 2, 224, 9, 2880);
        assert!(!is_fat32(&bpb));
        let geo = geometry16(&bpb).unwrap();
        assert_eq!(geo.kind, FatKind::Fat12);
        assert_eq!(geo.fat_start, 512);
        assert_eq!(geo.fat_size, 9 * 512);
        assert_eq!(
            geo.root,
            RootDir::Fixed {
                start: 19 * 512,
                size: 224 * 32
            }
        );
        assert_eq!(geo.data_start, 33 * 512);
        assert_eq!(geo.max_cluster, 2880 - 33 + 1);
        assert_eq!(geo.cluster_offset(2), Some(geo.data_start));
        assert_eq!(geo.cluster_offset(3), Some(geo.data_start + 512));
        assert_eq!(geo.cluster_offset(0), None);
        assert_eq!(geo.cluster_offset(1), None);
        assert_eq!(geo.cluster_offset(geo.max_cluster + 1), None);
    }

    #[test]
    fn many_clusters_is_fat16() {
        let geo = geometry16(&bpb(512, 4, 4, 2, 512, 64, 65_536)).unwrap();
        assert_eq!(geo.kind, FatKind::Fat16);
        assert_eq!(geo.cluster_size, 2048);
        assert_eq!(geo.max_cluster as u64, (65_536 - 4 - 128 - 32) / 4 + 1);
    }

    #[test]
    fn fat32_geometry_and_root_cluster() {
        let bpb = bpb(512, 8, 32, 2, 0, 0, 1 << 20);
        assert!(is_fat32(&bpb));
        let mut ext: RawBpbExt32 = bytemuck::Zeroable::zeroed();
        ext.sectors_per_fat_32.set(1024);
        ext.root_cluster.set(2);
        ext.signature_word.set(BOOT_SIGNATURE);
        assert_eq!(check_ext32(&bpb, &ext), Ok(()));
        let geo = geometry32(&bpb, &ext).unwrap();
        assert_eq!(geo.kind, FatKind::Fat32);
        assert_eq!(geo.root, RootDir::Cluster(2));
        assert_eq!(geo.data_start, (32 + 2048) * 512);
        assert_eq!(geo.max_cluster, ((1 << 20) - 32 - 2048) / 8 + 1);

        ext.root_cluster.set(1);
        assert!(matches!(
            geometry32(&bpb, &ext),
            Err(BootError::RootCluster { cluster: 1, .. })
        ));
        ext.version = [1, 0];
        assert!(matches!(
            check_ext32(&bpb, &ext),
            Err(BootError::Corrupt(_))
        ));
        ext.signature_word.set(0);
        assert_eq!(check_ext32(&bpb, &ext), Err(BootError::Signature(0)));
    }

    #[test]
    fn cluster_counts_are_limited_by_the_fat_type() {
        let fat16_max = bpb(512, 1, 1, 2, 512, 256, 1 + 512 + 32 + 65_524);
        let geo = geometry16(&fat16_max).unwrap();
        assert_eq!((geo.kind, geo.max_cluster), (FatKind::Fat16, 65_525));
        let fat16_over = bpb(512, 1, 1, 2, 512, 257, 1 + 514 + 32 + 65_525);
        assert!(matches!(
            geometry16(&fat16_over),
            Err(BootError::Corrupt(_))
        ));
        let fat12_max = bpb(512, 1, 1, 2, 512, 12, 1 + 24 + 32 + 4_084);
        assert_eq!(geometry16(&fat12_max).unwrap().kind, FatKind::Fat12);
        let no_root = bpb(512, 1, 1, 2, 0, 12, 1 + 24 + 4_000);
        assert!(!is_fat32(&no_root));
        assert!(matches!(geometry16(&no_root), Err(BootError::Corrupt(_))));

        let mut ext: RawBpbExt32 = bytemuck::Zeroable::zeroed();
        ext.root_cluster.set(2);
        ext.sectors_per_fat_32.set(1);
        let over = bpb(512, 1, 32, 1, 0, 0, u32::MAX);
        assert!(matches!(
            geometry32(&over, &ext),
            Err(BootError::Corrupt(_))
        ));
        let max = bpb(512, 1, 32, 1, 0, 0, 32 + 1 + FAT32_MAX_CLUSTERS);
        assert_eq!(
            geometry32(&max, &ext).unwrap().max_cluster,
            FAT32_MAX_CLUSTERS + 1
        );
        ext.sectors_per_fat_32.set(504);
        let small = bpb(512, 1, 32, 2, 0, 0, 65_536);
        let geo = geometry32(&small, &ext).unwrap();
        assert_eq!((geo.kind, geo.max_cluster), (FatKind::Fat32, 64_497));
    }

    #[test]
    fn ext16_checks_signature_before_fat_count() {
        let mut ext: RawBpbExt16 = bytemuck::Zeroable::zeroed();
        let bad_count = bpb(512, 1, 1, 3, 224, 9, 2880);
        assert_eq!(check_ext16(&bad_count, &ext), Err(BootError::Signature(0)));
        ext.signature_word = BOOT_SIGNATURE.to_le_bytes();
        assert!(matches!(
            check_ext16(&bad_count, &ext),
            Err(BootError::Corrupt(_))
        ));
        assert_eq!(check_ext16(&bpb(512, 1, 1, 2, 224, 9, 2880), &ext), Ok(()));
    }

    #[test]
    fn fs_info_signatures() {
        let mut info: RawFsInfo = bytemuck::Zeroable::zeroed();
        assert!(matches!(
            check_fs_info(&info),
            Err(BootError::FsInfoSignature {
                field: "FSI_LeadSig",
                ..
            })
        ));
        info.signature = FSINFO_LEAD_SIG.to_le_bytes();
        info.structure_signature = FSINFO_STRUC_SIG.to_le_bytes();
        assert!(matches!(
            check_fs_info(&info),
            Err(BootError::FsInfoSignature {
                field: "FSI_TrailSig",
                ..
            })
        ));
        info.trail_signature.set(FSINFO_TRAIL_SIG);
        assert_eq!(check_fs_info(&info), Ok(()));
    }
}

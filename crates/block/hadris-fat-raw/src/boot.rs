//! BIOS parameter block validation and volume geometry.

use hadris_fs::ErrorKind;

use crate::bpb::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};
use crate::entry::{
    FAT12_MAX_CLUSTERS, FAT16_MAX_CLUSTERS, FAT32_MAX_CLUSTERS, FIRST_DATA_CLUSTER, FatKind,
};

/// `FSI_LeadSig` of the FSInfo sector.
pub const FSINFO_LEAD_SIG: u32 = 0x4161_5252;
/// `FSI_StrucSig` of the FSInfo sector.
pub const FSINFO_STRUC_SIG: u32 = 0x6141_7272;
/// `FSI_TrailSig` of the FSInfo sector.
pub const FSINFO_TRAIL_SIG: u32 = 0xAA55_0000;
/// The signature word at bytes 510 and 511 of the boot sector.
pub const BOOT_SIGNATURE: u16 = 0xAA55;
/// Bytes of a boot sector [`parse_boot`] reads.
pub const BOOT_SECTOR_LEN: usize = 512;

const BPB_LEN: usize = size_of::<RawBpb>();
const FAT32_MIRRORING_DISABLED: u16 = 0x80;
const FAT32_ACTIVE_FAT: u16 = 0x0F;

/// Why a boot sector or FSInfo sector was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BootError {
    /// The sector and cluster sizes are not ones FAT allows, so the sector
    /// holds no FAT BIOS parameter block; the text names the field.
    NotFat(&'static str),
    /// A field violates the specification; the text names it.
    Corrupt(&'static str),
    /// The boot sector does not end in `0xAA55`.
    Signature(u16),
    /// The FAT32 root cluster is not a data cluster.
    RootCluster {
        /// The cluster the boot sector names.
        cluster: u32,
        /// The highest data cluster.
        max: u32,
    },
    /// An FSInfo signature is wrong.
    FsInfoSignature {
        /// The name of the field.
        field: &'static str,
        /// The value the specification requires.
        expected: u32,
        /// The value found.
        found: u32,
    },
}

impl BootError {
    /// [`ErrorKind::NotRecognized`] for [`NotFat`](Self::NotFat),
    /// [`ErrorKind::Corrupt`] for the rest.
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFat(_) => ErrorKind::NotRecognized,
            _ => ErrorKind::Corrupt,
        }
    }
}

/// Where the root directory lives. FAT has these two forms only, so the
/// enum is exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootLocation {
    /// FAT12/16: a fixed region of `size` bytes at byte `start`.
    Fixed {
        /// Byte offset of the region.
        start: u64,
        /// Its length in bytes.
        size: u64,
    },
    /// FAT32: an ordinary cluster chain from this cluster.
    Cluster(u32),
}

/// Byte layout of a FAT volume, from a boot sector [`parse_boot`] accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    kind: FatKind,
    sector_size: u32,
    cluster_size: u32,
    reserved_sectors: u16,
    fat_start: u64,
    fat_size: u64,
    fat_count: u8,
    active_fat: u8,
    mirrored: bool,
    root: RootLocation,
    data_start: u64,
    max_cluster: u32,
    fs_info_sector: Option<u16>,
    volume_serial: Option<u32>,
}

impl Geometry {
    /// The FAT variant.
    pub const fn kind(&self) -> FatKind {
        self.kind
    }

    /// Bytes per sector: 512, 1024, 2048 or 4096.
    pub const fn sector_size(&self) -> u32 {
        self.sector_size
    }

    /// Bytes per cluster, at most 32 KiB.
    pub const fn cluster_size(&self) -> u32 {
        self.cluster_size
    }

    /// Sectors before the first FAT.
    pub const fn reserved_sectors(&self) -> u16 {
        self.reserved_sectors
    }

    /// Byte offset of the first FAT copy.
    pub const fn fat_start(&self) -> u64 {
        self.fat_start
    }

    /// Bytes in one FAT copy.
    pub const fn fat_size(&self) -> u64 {
        self.fat_size
    }

    /// The number of FAT copies, 1 or 2.
    pub const fn fat_count(&self) -> u8 {
        self.fat_count
    }

    /// The copy that is read: 0 unless FAT32 mirroring is disabled.
    pub const fn active_fat(&self) -> u8 {
        self.active_fat
    }

    /// Whether every FAT copy is written, not only the active one.
    pub const fn mirrored(&self) -> bool {
        self.mirrored
    }

    /// The number of copies that are written: every copy when
    /// [`mirrored`](Self::mirrored), else the active one.
    pub const fn copies(&self) -> u8 {
        if self.mirrored { self.fat_count } else { 1 }
    }

    /// Byte offset of FAT copy `copy`.
    pub const fn fat_copy(&self, copy: u8) -> u64 {
        self.fat_start + copy as u64 * self.fat_size
    }

    /// Where the root directory lives.
    pub const fn root(&self) -> RootLocation {
        self.root
    }

    /// Byte offset of cluster 2, the first data cluster.
    pub const fn data_start(&self) -> u64 {
        self.data_start
    }

    /// Byte offset just past the last data cluster.
    pub const fn data_end(&self) -> u64 {
        self.data_start + (self.max_cluster - 1) as u64 * self.cluster_size as u64
    }

    /// The highest valid cluster number; clusters start at 2.
    pub const fn max_cluster(&self) -> u32 {
        self.max_cluster
    }

    /// The FAT32 FSInfo sector, when the boot sector names one within the
    /// reserved sectors.
    pub const fn fs_info_sector(&self) -> Option<u16> {
        self.fs_info_sector
    }

    /// The volume serial number, when the boot sector has an extended boot
    /// signature (`0x28` or `0x29`) that records one.
    pub const fn volume_serial(&self) -> Option<u32> {
        self.volume_serial
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn set_volume_serial(&mut self, serial: u32) {
        self.volume_serial = Some(serial);
    }

    /// Byte offset of `cluster`'s first byte, or `None` when `cluster` is
    /// not a data cluster.
    pub const fn cluster_offset(&self, cluster: u32) -> Option<u64> {
        if cluster < FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return None;
        }
        Some(self.data_start + (cluster - FIRST_DATA_CLUSTER) as u64 * self.cluster_size as u64)
    }
}

/// Checks a boot sector and returns the volume's geometry.
///
/// The BIOS parameter block must hold sector and cluster sizes FAT allows,
/// or the error is [`BootError::NotFat`]. The rest must describe a valid
/// FAT12, FAT16 or FAT32 layout whose FAT holds an entry for every cluster.
/// The variant follows from the cluster count, except that a volume with a
/// FAT32 layout and fewer than 65525 clusters is FAT32, as Linux and macOS
/// read it. The volume's size is not compared with a device.
pub fn parse_boot(sector: &[u8; BOOT_SECTOR_LEN]) -> Result<Geometry, BootError> {
    let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..BPB_LEN]);
    check_bpb(&bpb).map_err(|err| match err {
        BootError::Corrupt(field) => BootError::NotFat(field),
        other => other,
    })?;
    let geo = if is_fat32(&bpb) {
        let ext: RawBpbExt32 =
            bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
        check_ext32(&bpb, &ext)?;
        let mut geo = geometry32(&bpb, &ext)?;
        let flags = u16::from_le_bytes(ext.ext_flags);
        geo.mirrored = flags & FAT32_MIRRORING_DISABLED == 0;
        geo.active_fat = if geo.mirrored {
            0
        } else {
            (flags & FAT32_ACTIVE_FAT) as u8
        };
        let info = ext.fs_info_sector.get();
        geo.fs_info_sector = (info != 0 && info < geo.reserved_sectors).then_some(info);
        geo.volume_serial = serial(ext.ext_boot_signature, ext.volume_id);
        geo
    } else {
        let ext: RawBpbExt16 =
            bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt16>()]);
        check_ext16(&bpb, &ext)?;
        let mut geo = geometry16(&bpb)?;
        geo.volume_serial = serial(ext.ext_boot_signature, ext.volume_id);
        geo
    };
    if geo.max_cluster < FIRST_DATA_CLUSTER {
        return Err(BootError::Corrupt("volume has no data cluster"));
    }
    if geo.active_fat >= geo.fat_count {
        return Err(BootError::Corrupt(
            "BPB_ExtFlags names a FAT that does not exist",
        ));
    }
    let kind = geo.kind;
    if kind.entry_offset(geo.max_cluster as u64) + kind.entry_len() as u64 > geo.fat_size {
        return Err(BootError::Corrupt("FAT is too small for the cluster count"));
    }
    Ok(geo)
}

/// The serial an extended boot signature of `0x28` or `0x29` records.
fn serial(signature: u8, id: [u8; 4]) -> Option<u32> {
    matches!(signature, 0x28 | 0x29).then(|| u32::from_le_bytes(id))
}

/// Checks the fields common to every FAT variant: sector size, sectors per
/// cluster and cluster size.
fn check_bpb(bpb: &RawBpb) -> Result<(), BootError> {
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
fn is_fat32(bpb: &RawBpb) -> bool {
    bpb.root_entry_count == [0, 0] && bpb.sectors_per_fat_16 == [0, 0]
}

fn check_fat_count(bpb: &RawBpb) -> Result<(), BootError> {
    if bpb.fat_count != 1 && bpb.fat_count != 2 {
        return Err(BootError::Corrupt("BPB fat_count must be 1 or 2"));
    }
    Ok(())
}

/// Checks the FAT12/16 extended boot record.
fn check_ext16(bpb: &RawBpb, ext: &RawBpbExt16) -> Result<(), BootError> {
    let signature = u16::from_le_bytes(ext.signature_word);
    if signature != BOOT_SIGNATURE {
        return Err(BootError::Signature(signature));
    }
    check_fat_count(bpb)
}

/// Checks the FAT32 extended boot record.
fn check_ext32(bpb: &RawBpb, ext: &RawBpbExt32) -> Result<(), BootError> {
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
pub fn check_fs_info(info: &RawFsInfo) -> Result<(), BootError> {
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
fn geometry16(bpb: &RawBpb) -> Result<Geometry, BootError> {
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
        active_fat: 0,
        mirrored: true,
        reserved_sectors: reserved as u16,
        fs_info_sector: None,
        volume_serial: None,
        root: RootLocation::Fixed {
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
fn geometry32(bpb: &RawBpb, ext: &RawBpbExt32) -> Result<Geometry, BootError> {
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
        active_fat: 0,
        mirrored: true,
        reserved_sectors: reserved as u16,
        fs_info_sector: None,
        volume_serial: None,
        root: RootLocation::Cluster(root),
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
            RootLocation::Fixed {
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
        assert_eq!(geo.root, RootLocation::Cluster(2));
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

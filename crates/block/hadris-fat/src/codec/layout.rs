//! Volume layout for formatting: FAT variant, cluster size and FAT size
//! from the volume size.

use hadris_common::types::endian::{Endian, LittleEndian};
use hadris_common::types::number::{U16, U32};

use super::boot::{BOOT_SIGNATURE, FSINFO_LEAD_SIG, FSINFO_STRUC_SIG, FSINFO_TRAIL_SIG};
use super::entry::FatKind;
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};

const MIB: u64 = 1024 * 1024;
const MAX_CLUSTER_BYTES: u32 = 32 * 1024;
const MAX_CLUSTER_SECTORS: u32 = 128;
pub(crate) const FAT12_MAX_CLUSTERS: u32 = 4084;
pub(crate) const FAT16_MAX_CLUSTERS: u32 = 65524;
const FAT32_MAX_CLUSTERS: u32 = 0x0FFF_FFF5;
/// Largest volume formatted as FAT12 when the variant is not given.
const AUTO_FAT12_BELOW: u64 = 16 * MIB;
/// Smallest volume formatted as FAT32 when the variant is not given.
const AUTO_FAT32_FROM: u64 = 512 * MIB;

/// Why no layout fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutError {
    /// The volume is too small for the variant.
    TooSmall,
    /// The volume is too large for the variant.
    TooLarge,
    /// An option is out of range; the text names it.
    Invalid(&'static str),
}

/// What the caller asks for. `None` fields are chosen by [`plan`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Request {
    pub(crate) kind: Option<FatKind>,
    pub(crate) sector_size: u32,
    pub(crate) total_sectors: u64,
    pub(crate) cluster_size: Option<u32>,
    pub(crate) reserved_sectors: Option<u16>,
    pub(crate) fat_count: u8,
    pub(crate) root_entries: u16,
}

/// A volume layout in sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) kind: FatKind,
    pub(crate) sector_size: u32,
    pub(crate) cluster_sectors: u32,
    pub(crate) reserved_sectors: u16,
    pub(crate) fat_count: u8,
    /// Entries of the fixed root directory; 0 on FAT32.
    pub(crate) root_entries: u16,
    pub(crate) total_sectors: u32,
    pub(crate) fat_sectors: u32,
    pub(crate) clusters: u32,
}

impl Layout {
    pub(crate) fn root_sectors(&self) -> u32 {
        (self.root_entries as u32 * 32).div_ceil(self.sector_size)
    }

    pub(crate) fn fat_start(&self) -> u64 {
        self.reserved_sectors as u64 * self.sector_size as u64
    }

    /// Byte offset of the fixed root directory, or of the data region on
    /// FAT32.
    pub(crate) fn root_start(&self) -> u64 {
        self.fat_start() + self.fat_count as u64 * self.fat_sectors as u64 * self.sector_size as u64
    }

    pub(crate) fn data_start(&self) -> u64 {
        self.root_start() + self.root_sectors() as u64 * self.sector_size as u64
    }

    pub(crate) fn cluster_size(&self) -> u32 {
        self.cluster_sectors * self.sector_size
    }
}

/// The inclusive range of cluster counts of `kind`.
pub(crate) fn cluster_range(kind: FatKind) -> (u32, u32) {
    match kind {
        FatKind::Fat12 => (1, FAT12_MAX_CLUSTERS),
        FatKind::Fat16 => (FAT12_MAX_CLUSTERS + 1, FAT16_MAX_CLUSTERS),
        FatKind::Fat32 => (FAT16_MAX_CLUSTERS + 1, FAT32_MAX_CLUSTERS),
    }
}

/// Sectors per cluster for a volume of `bytes` with 512-byte sectors, after
/// the Microsoft defaults.
pub(crate) fn default_cluster_sectors(kind: FatKind, bytes: u64) -> u32 {
    let mib = bytes / MIB;
    let table: &[(u64, u32)] = match kind {
        FatKind::Fat12 => &[(2, 1), (4, 2), (8, 4), (16, 8)],
        FatKind::Fat16 => &[
            (8, 1),
            (16, 2),
            (32, 4),
            (64, 8),
            (128, 16),
            (256, 32),
            (512, 64),
        ],
        FatKind::Fat32 => &[
            (64, 1),
            (128, 2),
            (256, 4),
            (9 * 1024 - 1, 8),
            (17 * 1024 - 1, 16),
            (33 * 1024 - 1, 32),
        ],
    };
    let last = match kind {
        FatKind::Fat12 => 16,
        FatKind::Fat16 => 128,
        FatKind::Fat32 => 64,
    };
    table
        .iter()
        .find(|&&(limit, _)| mib <= limit)
        .map_or(last, |&(_, sectors)| sectors)
}

/// Sectors per FAT and cluster count when `available` sectors hold the FATs
/// and the data region: the smallest FAT that covers every cluster left
/// beside it.
pub(crate) fn fat_sectors(
    kind: FatKind,
    available: u32,
    cluster_sectors: u32,
    fat_count: u32,
    sector_size: u32,
) -> Option<(u32, u32)> {
    let mut fat = 1u32;
    loop {
        let data = available.saturating_sub(fat_count * fat);
        let clusters = data / cluster_sectors;
        let entries = clusters as u64 + 2;
        let bytes = match kind {
            FatKind::Fat12 => (entries * 3).div_ceil(2),
            FatKind::Fat16 => entries * 2,
            FatKind::Fat32 => entries * 4,
        };
        let needed = bytes.div_ceil(sector_size as u64);
        if needed <= fat as u64 {
            return Some((fat, clusters));
        }
        fat = u32::try_from(needed).ok()?;
        if fat.checked_mul(fat_count)? > available {
            return None;
        }
    }
}

/// Chooses the variant, when not given, and the cluster size, when not
/// given, so the cluster count fits the variant. A chosen cluster size
/// starts from [`default_cluster_sectors`] and doubles or halves until it
/// fits.
pub(crate) fn plan(request: &Request) -> Result<Layout, LayoutError> {
    let sector_size = request.sector_size;
    if !matches!(sector_size, 512 | 1024 | 2048 | 4096) {
        return Err(LayoutError::Invalid("sector size"));
    }
    if !matches!(request.fat_count, 1 | 2) {
        return Err(LayoutError::Invalid("FAT count"));
    }
    let total_sectors = u32::try_from(request.total_sectors).map_err(|_| LayoutError::TooLarge)?;
    let bytes = total_sectors as u64 * sector_size as u64;
    let kind = request.kind.unwrap_or(if bytes < AUTO_FAT12_BELOW {
        FatKind::Fat12
    } else if bytes < AUTO_FAT32_FROM {
        FatKind::Fat16
    } else {
        FatKind::Fat32
    });
    let reserved_sectors = match (kind, request.reserved_sectors) {
        (FatKind::Fat32, Some(reserved)) if reserved < 8 => {
            return Err(LayoutError::Invalid("reserved sectors"));
        }
        (_, Some(0)) => return Err(LayoutError::Invalid("reserved sectors")),
        (_, Some(reserved)) => reserved,
        (FatKind::Fat32, None) => 32,
        (_, None) => 1,
    };
    let root_entries = match kind {
        FatKind::Fat32 => 0,
        _ if request.root_entries == 0 => return Err(LayoutError::Invalid("root entries")),
        _ => {
            let per_sector = sector_size / 32;
            let entries = (request.root_entries as u32).div_ceil(per_sector) * per_sector;
            u16::try_from(entries).map_err(|_| LayoutError::Invalid("root entries"))?
        }
    };
    let root_sectors = (root_entries as u32 * 32).div_ceil(sector_size);
    let available = total_sectors
        .checked_sub(reserved_sectors as u32 + root_sectors)
        .filter(|&available| available > 0)
        .ok_or(LayoutError::TooSmall)?;
    let (min, max) = cluster_range(kind);
    let max_sectors = MAX_CLUSTER_SECTORS.min(MAX_CLUSTER_BYTES / sector_size);
    let fits = |cluster_sectors: u32| {
        let (fat, clusters) = fat_sectors(
            kind,
            available,
            cluster_sectors,
            request.fat_count as u32,
            sector_size,
        )
        .ok_or(LayoutError::TooSmall)?;
        if clusters < min {
            Err(LayoutError::TooSmall)
        } else if clusters > max {
            Err(LayoutError::TooLarge)
        } else {
            Ok((fat, clusters))
        }
    };
    let (cluster_sectors, (fat_sectors, clusters)) = match request.cluster_size {
        Some(size) => {
            if !size.is_power_of_two()
                || size % sector_size != 0
                || size / sector_size > max_sectors
            {
                return Err(LayoutError::Invalid("cluster size"));
            }
            let sectors = size / sector_size;
            (sectors, fits(sectors)?)
        }
        None => {
            let target = default_cluster_sectors(kind, bytes) * 512;
            let mut sectors = (target / sector_size).clamp(1, max_sectors);
            let mut grew = None;
            loop {
                match fits(sectors) {
                    Ok(found) => break (sectors, found),
                    Err(LayoutError::TooLarge) if sectors < max_sectors && grew != Some(false) => {
                        grew = Some(true);
                        sectors *= 2;
                    }
                    Err(LayoutError::TooSmall) if sectors > 1 && grew != Some(true) => {
                        grew = Some(false);
                        sectors /= 2;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    };
    Ok(Layout {
        kind,
        sector_size,
        cluster_sectors,
        reserved_sectors,
        fat_count: request.fat_count,
        root_entries,
        total_sectors,
        fat_sectors,
        clusters,
    })
}

/// The FSInfo sector's number on FAT32 volumes.
pub(crate) const FS_INFO_SECTOR: u16 = 1;
/// The backup boot sector's number on FAT32 volumes; the backup FSInfo
/// sector follows it.
pub(crate) const BACKUP_BOOT_SECTOR: u16 = 6;
/// The FAT32 root directory's cluster.
pub(crate) const ROOT_CLUSTER: u32 = 2;
/// `int 18h` to try the next boot device, then halt, for a volume that is
/// booted by mistake.
const BOOT_STUB: [u8; 5] = [0xCD, 0x18, 0xF4, 0xEB, 0xFD];
const NO_LABEL: [u8; 11] = *b"NO NAME    ";

/// Boot sector fields that do not follow from the layout.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BootFields {
    pub(crate) oem_name: [u8; 8],
    pub(crate) media: u8,
    pub(crate) hidden_sectors: u32,
    pub(crate) volume_id: u32,
    pub(crate) label: Option<[u8; 11]>,
}

/// The first 512 bytes of the boot sector.
pub(crate) fn encode_boot_sector(layout: &Layout, fields: &BootFields) -> [u8; 512] {
    let fat32 = layout.kind == FatKind::Fat32;
    let small_total = u16::try_from(layout.total_sectors).ok().filter(|_| !fat32);
    let code_start = if fat32 { 0x5A } else { 0x3E };
    let bpb = RawBpb {
        jump: [0xEB, code_start - 2, 0x90],
        oem_name: fields.oem_name,
        bytes_per_sector: U16::<LittleEndian>::new(layout.sector_size as u16),
        sectors_per_cluster: layout.cluster_sectors as u8,
        reserved_sector_count: U16::<LittleEndian>::new(layout.reserved_sectors),
        fat_count: layout.fat_count,
        root_entry_count: layout.root_entries.to_le_bytes(),
        total_sectors_16: small_total.unwrap_or(0).to_le_bytes(),
        media_type: fields.media,
        sectors_per_fat_16: if fat32 { 0 } else { layout.fat_sectors as u16 }.to_le_bytes(),
        sectors_per_track: 63u16.to_le_bytes(),
        num_heads: 255u16.to_le_bytes(),
        hidden_sector_count: fields.hidden_sectors.to_le_bytes(),
        total_sectors_32: if small_total.is_some() {
            0
        } else {
            layout.total_sectors
        }
        .to_le_bytes(),
    };
    let drive_number = if fields.media == 0xF8 { 0x80 } else { 0x00 };
    let label = fields.label.unwrap_or(NO_LABEL);
    let mut out = [0u8; 512];
    out[..size_of::<RawBpb>()].copy_from_slice(bytemuck::bytes_of(&bpb));
    let ext = &mut out[size_of::<RawBpb>()..];
    if fat32 {
        let ext32 = RawBpbExt32 {
            sectors_per_fat_32: U32::<LittleEndian>::new(layout.fat_sectors),
            ext_flags: [0, 0],
            version: [0, 0],
            root_cluster: U32::<LittleEndian>::new(ROOT_CLUSTER),
            fs_info_sector: U16::<LittleEndian>::new(FS_INFO_SECTOR),
            boot_sector: BACKUP_BOOT_SECTOR.to_le_bytes(),
            reserved: [0; 12],
            drive_number,
            reserved1: 0,
            ext_boot_signature: 0x29,
            volume_id: fields.volume_id.to_le_bytes(),
            volume_label: label,
            fs_type: *b"FAT32   ",
            padding1: [0; 420],
            signature_word: U16::<LittleEndian>::new(BOOT_SIGNATURE),
        };
        ext.copy_from_slice(bytemuck::bytes_of(&ext32));
    } else {
        let ext16 = RawBpbExt16 {
            drive_number,
            reserved1: 0,
            ext_boot_signature: 0x29,
            volume_id: fields.volume_id.to_le_bytes(),
            volume_label: label,
            fs_type: if layout.kind == FatKind::Fat12 {
                *b"FAT12   "
            } else {
                *b"FAT16   "
            },
            padding1: [0; 448],
            signature_word: BOOT_SIGNATURE.to_le_bytes(),
        };
        ext.copy_from_slice(bytemuck::bytes_of(&ext16));
    }
    let code_start = code_start as usize;
    out[code_start..code_start + BOOT_STUB.len()].copy_from_slice(&BOOT_STUB);
    out
}

/// The FAT32 FSInfo sector with `free` clusters and the search hint `next`.
pub(crate) fn encode_fs_info(free: u32, next: u32) -> [u8; 512] {
    let info = RawFsInfo {
        signature: FSINFO_LEAD_SIG.to_le_bytes(),
        reserved1: [0; 480],
        structure_signature: FSINFO_STRUC_SIG.to_le_bytes(),
        free_count: U32::<LittleEndian>::new(free),
        next_free: U32::<LittleEndian>::new(next),
        reserved2: [0; 12],
        trail_signature: U32::<LittleEndian>::new(FSINFO_TRAIL_SIG),
    };
    let mut out = [0u8; 512];
    out.copy_from_slice(bytemuck::bytes_of(&info));
    out
}

/// The reserved entries at the start of each FAT: the media byte, an
/// end-of-chain entry and, on FAT32, the root directory's cluster. Returns
/// the bytes and how many of them to write.
pub(crate) fn reserved_fat_entries(kind: FatKind, media: u8) -> ([u8; 12], usize) {
    let mask = kind.mask();
    let mut entries = [0u8; 12];
    let mut values = [(0u64, (mask & !0xFF) | media as u32), (1, mask), (2, 0)];
    let mut count = 2;
    if kind == FatKind::Fat32 {
        values[2] = (ROOT_CLUSTER as u64, kind.end_of_chain());
        count = 3;
    }
    for &(cluster, value) in &values[..count] {
        let at = kind.entry_offset(cluster) as usize;
        kind.encode(cluster, value, &mut entries[at..]);
    }
    let len = kind.entry_offset(count as u64 - 1) as usize + kind.entry_len();
    (entries, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: Option<FatKind>, bytes: u64) -> Request {
        Request {
            kind,
            sector_size: 512,
            total_sectors: bytes / 512,
            cluster_size: None,
            reserved_sectors: None,
            fat_count: 2,
            root_entries: 512,
        }
    }

    #[test]
    fn floppy_geometry() {
        let layout = plan(&Request {
            root_entries: 224,
            cluster_size: Some(512),
            ..request(Some(FatKind::Fat12), 1_474_560)
        })
        .unwrap();
        assert_eq!(layout.fat_sectors, 9);
        assert_eq!(layout.clusters, 2847);
        assert_eq!(layout.root_sectors(), 14);
        assert_eq!(layout.data_start(), 33 * 512);
    }

    #[test]
    fn auto_selects_by_size() {
        let kind = |bytes| plan(&request(None, bytes)).unwrap().kind;
        assert_eq!(kind(MIB), FatKind::Fat12);
        assert_eq!(kind(16 * MIB - 512), FatKind::Fat12);
        assert_eq!(kind(16 * MIB), FatKind::Fat16);
        assert_eq!(kind(512 * MIB - 512), FatKind::Fat16);
        assert_eq!(kind(512 * MIB), FatKind::Fat32);
        assert_eq!(kind(2048 * MIB), FatKind::Fat32);
    }

    #[test]
    fn cluster_size_adjusts_to_the_variant() {
        let fat12 = plan(&request(Some(FatKind::Fat12), 32 * MIB)).unwrap();
        assert!(fat12.clusters <= FAT12_MAX_CLUSTERS);
        assert_eq!(fat12.cluster_size(), 16 * 1024);
        let fat16 = plan(&request(Some(FatKind::Fat16), 2000 * MIB)).unwrap();
        assert_eq!(fat16.cluster_size(), MAX_CLUSTER_BYTES);
        let fat16 = plan(&request(Some(FatKind::Fat16), 4 * MIB)).unwrap();
        assert!(fat16.clusters > FAT12_MAX_CLUSTERS);
    }

    #[test]
    fn limits() {
        assert_eq!(
            plan(&request(Some(FatKind::Fat32), 32 * MIB)),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(
            plan(&request(Some(FatKind::Fat16), 2 * MIB)),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(
            plan(&request(Some(FatKind::Fat12), 256 * MIB)),
            Err(LayoutError::TooLarge)
        );
        assert_eq!(
            plan(&request(Some(FatKind::Fat16), 4096 * MIB)),
            Err(LayoutError::TooLarge)
        );
        assert_eq!(plan(&request(None, 33 * 512)), Err(LayoutError::TooSmall));
        assert!(plan(&request(None, 36 * 512)).is_ok());
        assert_eq!(
            plan(&request(None, (u32::MAX as u64 + 1) * 512)),
            Err(LayoutError::TooLarge)
        );
    }

    #[test]
    fn rejects_bad_options() {
        let bad = |request: Request| matches!(plan(&request), Err(LayoutError::Invalid(_)));
        let base = request(None, 8 * MIB);
        assert!(bad(Request {
            sector_size: 256,
            ..base
        }));
        assert!(bad(Request {
            fat_count: 3,
            ..base
        }));
        assert!(bad(Request {
            cluster_size: Some(3 * 512),
            ..base
        }));
        assert!(bad(Request {
            cluster_size: Some(256),
            ..base
        }));
        assert!(bad(Request {
            cluster_size: Some(64 * 1024),
            ..base
        }));
        assert!(bad(Request {
            root_entries: 0,
            ..base
        }));
        assert!(bad(Request {
            reserved_sectors: Some(0),
            ..base
        }));
        assert!(bad(Request {
            kind: Some(FatKind::Fat32),
            reserved_sectors: Some(7),
            ..request(None, 64 * MIB)
        }));
    }

    #[test]
    fn root_entries_fill_whole_sectors() {
        let layout = plan(&Request {
            root_entries: 20,
            ..request(None, 4 * MIB)
        })
        .unwrap();
        assert_eq!(layout.root_entries, 32);
        let layout = plan(&Request {
            root_entries: 20,
            sector_size: 4096,
            total_sectors: 1024,
            ..request(None, 0)
        })
        .unwrap();
        assert_eq!(layout.root_entries, 128);
    }

    fn round_trip(layout: &Layout) {
        let fields = BootFields {
            oem_name: *b"TESTOEM ",
            media: 0xF8,
            hidden_sectors: 63,
            volume_id: 0x1234_5678,
            label: Some(*b"LABEL      "),
        };
        let sector = encode_boot_sector(layout, &fields);
        let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..size_of::<RawBpb>()]);
        let ext = &sector[size_of::<RawBpb>()..];
        super::super::boot::check_bpb(&bpb).unwrap();
        let geometry = if layout.kind == FatKind::Fat32 {
            let ext: RawBpbExt32 = bytemuck::pod_read_unaligned(ext);
            super::super::boot::check_ext32(&bpb, &ext).unwrap();
            assert_eq!(ext.volume_label, *b"LABEL      ");
            super::super::boot::geometry32(&bpb, &ext).unwrap()
        } else {
            let ext: RawBpbExt16 = bytemuck::pod_read_unaligned(ext);
            super::super::boot::check_ext16(&bpb, &ext).unwrap();
            assert_eq!(u32::from_le_bytes(ext.volume_id), 0x1234_5678);
            super::super::boot::geometry16(&bpb).unwrap()
        };
        assert_eq!(geometry.kind, layout.kind);
        assert_eq!(geometry.data_start, layout.data_start());
        assert_eq!(geometry.max_cluster, layout.clusters + 1);
        assert_eq!(geometry.cluster_size, layout.cluster_size());
        assert!(
            layout.kind.entry_offset(geometry.max_cluster as u64) + layout.kind.entry_len() as u64
                <= geometry.fat_size
        );
    }

    #[test]
    fn boot_sectors_parse_back() {
        for (kind, bytes) in [
            (None, 36 * 512),
            (Some(FatKind::Fat12), 16 * MIB),
            (None, 64 * MIB),
            (Some(FatKind::Fat32), 40 * MIB),
            (None, 8 * 1024 * MIB),
        ] {
            round_trip(&plan(&request(kind, bytes)).unwrap());
        }
        let layout = plan(&Request {
            sector_size: 4096,
            total_sectors: 16 * MIB / 4096,
            ..request(Some(FatKind::Fat12), 0)
        })
        .unwrap();
        round_trip(&layout);
    }

    #[test]
    fn reserved_entries() {
        assert_eq!(
            reserved_fat_entries(FatKind::Fat12, 0xF0),
            ([0xF0, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0], 3)
        );
        assert_eq!(reserved_fat_entries(FatKind::Fat16, 0xF8).1, 4);
        assert_eq!(
            reserved_fat_entries(FatKind::Fat32, 0xF8),
            (
                [
                    0xF8, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F, 0xF8, 0xFF, 0xFF, 0x0F
                ],
                12
            )
        );
    }
}

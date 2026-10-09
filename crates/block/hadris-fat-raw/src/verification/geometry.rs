use crate::{FatKind, Geometry, RootLocation, exfat, layout};

fn fat_sector(fat32: bool) -> [u8; 512] {
    let mut sector: [u8; 512] = kani::any();
    let shift: u8 = kani::any();
    kani::assume(shift < 4);
    sector[11..13].copy_from_slice(&(512u16 << shift).to_le_bytes());
    sector[510..512].copy_from_slice(&0xaa55u16.to_le_bytes());
    if fat32 {
        sector[17..19].fill(0);
        sector[22..24].fill(0);
        sector[42..44].fill(0);
    } else {
        kani::assume(u16::from_le_bytes([sector[17], sector[18]]) != 0);
    }
    sector
}

fn check_fat_bounds(sector: &[u8; 512], geo: &Geometry) {
    let small = u16::from_le_bytes([sector[19], sector[20]]);
    let total = if small != 0 {
        u64::from(small)
    } else {
        u64::from(u32::from_le_bytes(sector[32..36].try_into().unwrap()))
    };
    let volume = total * u64::from(geo.sector_size());
    assert_eq!(
        geo.sector_size(),
        u32::from(u16::from_le_bytes([sector[11], sector[12]]))
    );
    assert_eq!(
        geo.cluster_size(),
        geo.sector_size() * u32::from(sector[13])
    );
    assert!(geo.data_end() <= volume);
    assert!(geo.max_cluster() >= 2);
    assert_eq!(geo.cluster_size(), 1 << geo.cluster_shift());
    assert!(geo.active_fat() < geo.fat_count());
    let cluster: u32 = kani::any();
    let offset = geo.cluster_offset(cluster);
    assert_eq!(
        offset.is_some(),
        cluster >= 2 && cluster <= geo.max_cluster()
    );
    if let Some(offset) = offset {
        assert!(offset >= geo.data_start());
        assert!(offset + u64::from(geo.cluster_size()) <= volume);
        let entry_end = match geo.kind() {
            FatKind::Fat12 => u64::from(cluster) * 3 / 2 + 2,
            FatKind::Fat16 => u64::from(cluster) * 2 + 2,
            FatKind::Fat32 => u64::from(cluster) * 4 + 4,
        };
        assert!(entry_end <= geo.fat_size());
    }
    let copy: u8 = kani::any();
    if copy < geo.fat_count() {
        assert!(geo.fat_copy(copy) + geo.fat_size() <= geo.data_start());
    }
}

#[kani::proof]
fn fat16_accepted_geometry_is_bounded() {
    let sector = fat_sector(false);
    if let Ok(geo) = crate::parse_boot(&sector) {
        kani::cover!(true, "accepted FAT12/16 geometry");
        check_fat_bounds(&sector, &geo);
        let clusters = geo.max_cluster() - 1;
        assert_eq!(
            geo.kind(),
            if clusters < 4085 {
                FatKind::Fat12
            } else {
                FatKind::Fat16
            }
        );
        assert!(clusters < 65525);
        let RootLocation::Fixed { start, size } = geo.root() else {
            panic!("fixed root required");
        };
        assert!(start + size <= geo.data_start());
        assert_eq!(
            size,
            u64::from(u16::from_le_bytes([sector[17], sector[18]])) * 32
        );
    }
}

#[kani::proof]
fn fat32_accepted_geometry_is_bounded() {
    let sector = fat_sector(true);
    if let Ok(geo) = crate::parse_boot(&sector) {
        kani::cover!(true, "accepted FAT32 geometry");
        check_fat_bounds(&sector, &geo);
        assert_eq!(geo.kind(), FatKind::Fat32);
        let RootLocation::Cluster(root) = geo.root() else {
            panic!("cluster root required");
        };
        assert!(geo.cluster_offset(root).is_some());
        assert!(geo.max_cluster() <= 0x0fff_fff6);
        let flags = u16::from_le_bytes([sector[40], sector[41]]);
        assert_eq!(geo.mirrored(), flags & 0x80 == 0);
        assert_eq!(
            geo.active_fat(),
            if flags & 0x80 == 0 {
                0
            } else {
                (flags & 15) as u8
            }
        );
    }
}

#[kani::proof]
fn fat_invalid_cluster_sizes_are_rejected() {
    let sector: [u8; 512] = kani::any();
    let bytes = u32::from(u16::from_le_bytes([sector[11], sector[12]]));
    let clusters = u32::from(sector[13]);
    let valid_sector = matches!(bytes, 512 | 1024 | 2048 | 4096);
    let valid_cluster = matches!(clusters, 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128);
    if !valid_sector || !valid_cluster || bytes * clusters > 65536 {
        assert!(crate::parse_boot(&sector).is_err());
    }
}

#[kani::proof]
#[kani::unwind(54)]
fn exfat_accepted_geometry_is_bounded() {
    let bytes: [u8; 512] = kani::any();
    let mut boot: exfat::BootSector = bytemuck::pod_read_unaligned(&bytes);
    boot.file_system_name = *b"EXFAT   ";
    boot.must_be_zero.fill(0);
    boot.file_system_revision.set(0x0100);
    boot.boot_signature.set(0xaa55);
    if let Ok(geo) = exfat::parse_boot(&boot) {
        kani::cover!(true, "accepted exFAT geometry");
        assert!((9..=12).contains(&geo.sector_shift()));
        assert!(geo.cluster_shift() <= 25);
        let heap_end = u64::from(geo.cluster_count()) * geo.cluster_size()
            + u64::from(boot.cluster_heap_offset.get()) * geo.sector_size();
        assert!(heap_end <= geo.volume_len());
        assert!(
            geo.fat_start() + u64::from(boot.fat_length.get()) * geo.sector_size()
                <= u64::from(boot.cluster_heap_offset.get()) * geo.sector_size()
        );
        assert!(
            u64::from(geo.cluster_count()) + 2
                <= u64::from(boot.fat_length.get()) * geo.sector_size() / 4
        );
        assert!(geo.is_cluster(geo.root()));
        let cluster: u32 = kani::any();
        let offset = geo.cluster_offset(cluster);
        assert_eq!(
            offset.is_some(),
            cluster >= 2 && cluster <= geo.max_cluster()
        );
        if let Some(offset) = offset {
            assert!(offset + geo.cluster_size() <= geo.volume_len());
            assert_eq!(geo.cluster_of(offset), Some(cluster));
            assert_eq!(
                geo.cluster_of(offset + geo.cluster_size() - 1),
                Some(cluster)
            );
        }
        if let Some(mirror) = geo.mirror_fat() {
            assert!(mirror + u64::from(boot.fat_length.get()) * geo.sector_size() <= heap_end);
            assert_eq!(geo.active(), (boot.volume_flags.get() & 1) as u8);
        } else {
            assert_eq!(geo.active(), 0);
        }
    }
}

#[kani::proof]
#[kani::unwind(4)]
fn explicit_fat_layout_fits_volume() {
    let selector: u8 = kani::any();
    kani::assume(selector < 3);
    let kind = match selector {
        0 => FatKind::Fat12,
        1 => FatKind::Fat16,
        _ => FatKind::Fat32,
    };
    let sectors: u32 = kani::any();
    let request = layout::Request::new(512, u64::from(sectors))
        .with_kind(kind)
        .with_cluster_size(512);
    if let Ok(plan) = layout::plan(&request) {
        kani::cover!(true, "accepted explicit formatter layout");
        assert!(plan.data_start() + u64::from(plan.clusters()) * 512 <= u64::from(sectors) * 512);
        let entries = u64::from(plan.clusters()) + 2;
        let required = match kind {
            FatKind::Fat12 => (entries * 3 + 1) / 2,
            FatKind::Fat16 => entries * 2,
            FatKind::Fat32 => entries * 4,
        };
        assert!(required <= u64::from(plan.fat_sectors()) * 512);
        let (low, high) = match kind {
            FatKind::Fat12 => (1, 4084),
            FatKind::Fat16 => (4085, 65524),
            FatKind::Fat32 => (65525, 0x0fff_fff5),
        };
        assert!((low..=high).contains(&plan.clusters()));
    }
}

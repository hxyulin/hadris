//! `format` checked against the host `fsck`, the on-disk layout and `FatFs`.

#[path = "common/fatfs.rs"]
mod common;

use common::{CASES, fsck};
use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FatKind, FormatOptions, MountOptions, VolumeLabel};
use hadris_fs::sync::{DriverExt, FileSystem, PathExt, Volume};
use hadris_fs::{Clock, DateTime, ErrorKind, HeapTable, NoClock};
use hadris_storage::{BlockSize, MemDevice};

const MIB: u64 = 1024 * 1024;

fn device(bytes: u64, block: u32) -> MemDevice<Vec<u8>> {
    MemDevice::new(vec![0; bytes as usize], BlockSize::new(block).unwrap())
}

fn label(text: &str) -> VolumeLabel {
    VolumeLabel::new(text).unwrap()
}

fn formatted(bytes: u64, block: u32, options: FormatOptions) -> Vec<u8> {
    format(device(bytes, block), options)
        .unwrap()
        .into_inner()
        .into_inner()
}

fn error(bytes: u64, block: u32, options: FormatOptions) -> ErrorKind {
    format(device(bytes, block), options).unwrap_err().kind()
}

/// Writes files through `FatFs`, then reads them back through a fresh mount
/// and runs the host `fsck`.
fn exercise(image: Vec<u8>, block: u32, expected: FatKind, name: &str) -> Vec<u8> {
    let dev = MemDevice::new(image, BlockSize::new(block).unwrap());
    let fs = FatFs::open_with(dev, MountOptions::new().with_table(HeapTable::new())).unwrap();
    assert_eq!(fs.kind(), expected, "{name}");
    let vol = Volume::new(fs);
    let payload = common::payload(20_000, 3);
    vol.create_dir_all("/Some Dir/inner").unwrap();
    vol.write_file("/Some Dir/inner/data.bin", &payload)
        .unwrap();
    vol.write_file("/README.TXT", b"formatted").unwrap();
    vol.sync().unwrap();
    let image = vol.into_inner().into_inner().into_inner();
    let dev = MemDevice::new(image.clone(), BlockSize::new(block).unwrap());
    let mut fs = FatFs::open_with(dev, MountOptions::new().with_table(HeapTable::new())).unwrap();
    assert!(
        fs.metadata("/some dir").unwrap().file_type().is_dir(),
        "{name}"
    );
    assert_eq!(
        fs.read_to_vec("/readme.txt").unwrap(),
        b"formatted",
        "{name}"
    );
    assert_eq!(
        fs.read_to_vec("/Some Dir/inner/data.bin").unwrap(),
        payload,
        "{name}"
    );
    let ran = fsck(&image, name);
    eprintln!("{name}: {ran} host fsck tools passed");
    image
}

#[test]
fn formats_every_kind_that_mounts_and_passes_fsck() {
    let cases: [(&str, u64, u32, FormatOptions, FatKind); 8] = [
        ("auto 1 MiB", MIB, 512, FormatOptions::new(), FatKind::Fat12),
        (
            "auto 8 MiB",
            8 * MIB,
            512,
            FormatOptions::new(),
            FatKind::Fat12,
        ),
        (
            "auto below 16 MiB",
            16 * MIB - 512,
            512,
            FormatOptions::new(),
            FatKind::Fat12,
        ),
        (
            "auto 16 MiB",
            16 * MIB,
            512,
            FormatOptions::new(),
            FatKind::Fat16,
        ),
        (
            "fat32 40 MiB",
            40 * MIB,
            512,
            FormatOptions::new().with_kind(FatKind::Fat32),
            FatKind::Fat32,
        ),
        (
            "fat12 4096-byte sectors on 512-byte blocks",
            4 * MIB,
            512,
            FormatOptions::new().with_sector_size(4096),
            FatKind::Fat12,
        ),
        (
            "fat16 on 4096-byte blocks",
            32 * MIB,
            4096,
            FormatOptions::new(),
            FatKind::Fat16,
        ),
        (
            "fat32 with one FAT and 2 KiB clusters",
            160 * MIB,
            512,
            FormatOptions::new()
                .with_kind(FatKind::Fat32)
                .with_fat_count(1)
                .with_cluster_size(2048),
            FatKind::Fat32,
        ),
    ];
    for (name, bytes, block, options, kind) in cases {
        let image = formatted(bytes, block, options.with_label(label("hadris")));
        assert_eq!(bpb_label(&image), b"HADRIS     ", "{name}");
        let dev = MemDevice::new(image.clone(), BlockSize::new(block).unwrap());
        let label = FatFs::open(dev).unwrap().label().unwrap().unwrap();
        assert_eq!(label.as_str(), "HADRIS", "{name}");
        exercise(image, block, kind, name);
    }
}

#[test]
fn defaults_follow_the_device_block_size() {
    for block in [512, 1024, 2048, 4096] {
        let image = formatted(8 * MIB, block, FormatOptions::new());
        assert_eq!(u16::from_le_bytes([image[11], image[12]]) as u32, block);
    }
    let image = formatted(8 * MIB, 256, FormatOptions::new());
    assert_eq!(u16::from_le_bytes([image[11], image[12]]), 512);
    exercise(image, 256, FatKind::Fat12, "256-byte blocks");
    assert_eq!(
        error(8 * MIB, 8192, FormatOptions::new()),
        ErrorKind::Unsupported
    );
}

/// The volume label in the boot sector.
fn bpb_label(image: &[u8]) -> &[u8] {
    let fat32 = u16::from_le_bytes([image[22], image[23]]) == 0;
    let at = if fat32 { 71 } else { 43 };
    &image[at..at + 11]
}

#[test]
fn boot_sector_fats_and_fsinfo_are_consistent() {
    for case in CASES {
        let options = FormatOptions::new()
            .with_kind(case.kind)
            .with_sector_size(case.sector)
            .with_label(label("HADRIS"))
            .with_volume_id(0xC0FF_EE00);
        let image = formatted(case.size, case.block, options);
        let sector = case.sector as usize;
        let fat32 = case.kind == FatKind::Fat32;
        assert_eq!(image[510..512], [0x55, 0xAA], "{}: signature", case.name);
        assert_eq!(
            u16::from_le_bytes([image[11], image[12]]) as usize,
            sector,
            "{}: sector size",
            case.name
        );
        let id_at = if fat32 { 67 } else { 39 };
        assert_eq!(
            u32::from_le_bytes(image[id_at..id_at + 4].try_into().unwrap()),
            0xC0FF_EE00,
            "{}: volume id",
            case.name
        );
        assert_eq!(bpb_label(&image), b"HADRIS     ", "{}", case.name);
        let reserved = u16::from_le_bytes([image[14], image[15]]) as usize;
        let fats = image[16] as usize;
        let fat_sectors = if fat32 {
            u32::from_le_bytes(image[36..40].try_into().unwrap()) as usize
        } else {
            u16::from_le_bytes([image[22], image[23]]) as usize
        };
        let fat_len = fat_sectors * sector;
        let fat_start = reserved * sector;
        let first = &image[fat_start..fat_start + fat_len];
        for copy in 1..fats {
            let at = fat_start + copy * fat_len;
            assert_eq!(&image[at..at + fat_len], first, "{}: FAT copy", case.name);
        }
        let media = image[21];
        let (used, eoc) = match case.kind {
            FatKind::Fat12 => (3, &[media, 0xFF, 0xFF][..]),
            FatKind::Fat16 => (4, &[media, 0xFF, 0xFF, 0xFF][..]),
            _ => (12, &[media, 0xFF, 0xFF][..]),
        };
        assert_eq!(&first[..eoc.len()], eoc, "{}: reserved entries", case.name);
        if fat32 {
            assert!(
                u32::from_le_bytes(first[8..12].try_into().unwrap()) & 0x0FFF_FFFF >= 0x0FFF_FFF8,
                "{}: root cluster",
                case.name
            );
        }
        assert!(
            first[used..].iter().all(|&byte| byte == 0),
            "{}: free FAT",
            case.name
        );
        let root = fat_start + fats * fat_len;
        assert_eq!(&image[root..root + 11], b"HADRIS     ");
        if fat32 {
            let info = &image[sector..2 * sector];
            assert_eq!(&info[..4], b"RRaA", "FSInfo");
            assert_eq!(&info[484..488], b"rrAa", "FSInfo");
            assert_eq!(info[508..512], [0, 0, 0x55, 0xAA], "FSInfo");
            let dev = MemDevice::new(image.clone(), BlockSize::new(case.block).unwrap());
            let free = FatFs::open(dev).unwrap().stats().unwrap().free_blocks();
            assert_eq!(
                u32::from_le_bytes(info[488..492].try_into().unwrap()) as u64,
                free,
                "FSInfo free count"
            );
            assert_eq!(
                image[..sector],
                image[6 * sector..7 * sector],
                "backup boot sector"
            );
            assert_eq!(
                image[sector..2 * sector],
                image[7 * sector..8 * sector],
                "backup FSInfo"
            );
        }
        fsck(&image, case.name);
    }
}

#[test]
fn reproducible_and_clocked() {
    let a = formatted(4 * MIB, 512, FormatOptions::new().with_label(label("A")));
    let b = formatted(4 * MIB, 512, FormatOptions::new().with_label(label("A")));
    assert_eq!(a, b);
    let id = u32::from_le_bytes(a[39..43].try_into().unwrap());

    #[derive(Clone, Copy)]
    struct Fixed(DateTime);
    impl Clock for Fixed {
        fn now(&self) -> DateTime {
            self.0
        }
    }
    let time = DateTime::from_unix_seconds(1_900_000_000).unwrap();
    let fs = format(
        device(4 * MIB, 512),
        FormatOptions::new()
            .with_label(label("A"))
            .with_clock(Fixed(time)),
    )
    .unwrap();
    assert_eq!(fs.clock().now(), time);
    let c = fs.into_inner().into_inner();
    assert_ne!(u32::from_le_bytes(c[39..43].try_into().unwrap()), id);
    let root = c
        .chunks_exact(32)
        .find(|entry| &entry[..11] == b"A          ")
        .unwrap();
    assert_ne!(
        root[22..26],
        [0, 0, 0, 0],
        "label entry has the clock's time"
    );
    assert_eq!(NoClock.now().unix_seconds(), 315_532_800);
    let defaults = format(
        device(4 * MIB, 512),
        FormatOptions::default().with_label(label("A")),
    )
    .unwrap();
    assert_eq!(defaults.into_inner().into_inner(), a);
}

#[test]
fn floppy_geometry() {
    let options = FormatOptions::new()
        .with_kind(FatKind::Fat12)
        .with_cluster_size(512)
        .with_root_entries(224)
        .with_media(0xF0)
        .with_oem_name(*b"MSDOS5.0");
    let image = formatted(1_474_560, 512, options);
    assert_eq!(&image[3..11], b"MSDOS5.0");
    assert_eq!(image[13], 1, "sectors per cluster");
    assert_eq!(u16::from_le_bytes([image[17], image[18]]), 224);
    assert_eq!(u16::from_le_bytes([image[19], image[20]]), 2880);
    assert_eq!(image[21], 0xF0);
    assert_eq!(u16::from_le_bytes([image[22], image[23]]), 9);
    assert_eq!(image[36], 0, "drive number of removable media");
    assert_eq!(image[512..515], [0xF0, 0xFF, 0xFF]);
    exercise(image, 512, FatKind::Fat12, "floppy");
}

/// The smallest device, in sectors, that `options` formats, with the error
/// one sector below it.
fn smallest(options: &FormatOptions, from: u64, to: u64) -> (u64, ErrorKind) {
    let mut buffer = vec![0u8; to as usize * 512];
    let (mut low, mut high) = (from, to);
    let mut attempt = |sectors: u64| {
        let dev = MemDevice::new(
            &mut buffer[..sectors as usize * 512],
            BlockSize::new(512).unwrap(),
        );
        format(dev, options.clone())
            .map(|_| ())
            .map_err(|err| err.kind())
    };
    assert!(attempt(high).is_ok());
    let below = attempt(low).unwrap_err();
    while high - low > 1 {
        let mid = (low + high) / 2;
        if attempt(mid).is_ok() {
            high = mid;
        } else {
            low = mid;
        }
    }
    (high, below)
}

#[test]
fn smallest_volumes_and_kind_boundaries() {
    let (fat12, below) = smallest(&FormatOptions::new(), 1, 64);
    assert_eq!(fat12, 36);
    assert_eq!(below, ErrorKind::NoSpace);
    assert_eq!(
        error(35 * 512, 512, FormatOptions::new()),
        ErrorKind::NoSpace
    );
    let dev = MemDevice::new(
        formatted(36 * 512, 512, FormatOptions::new()),
        BlockSize::new(512).unwrap(),
    );
    let vol = Volume::new(FatFs::open(dev).unwrap());
    assert_eq!(vol.stats().unwrap().free_blocks(), 1);
    vol.write_file("/ONE.TXT", b"1").unwrap();
    assert_eq!(
        vol.write_file("/TWO.TXT", b"2").unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    vol.sync().unwrap();
    let image = vol.into_inner().into_inner().into_inner();
    assert_eq!(bpb_label(&image), b"NO NAME    ");
    fsck(&image, "smallest");
    let tiny = FormatOptions::new().with_root_entries(16);
    let image = formatted(64 * 512, 512, tiny);
    let fs = FatFs::open(MemDevice::new(image.clone(), BlockSize::new(512).unwrap())).unwrap();
    assert_eq!(fs.kind(), FatKind::Fat12);
    fsck(&image, "64 sectors");

    let fat16 = FormatOptions::new().with_kind(FatKind::Fat16);
    let (sectors, below) = smallest(&fat16, 4000, 8192);
    assert_eq!(below, ErrorKind::NoSpace);
    let image = formatted(sectors * 512, 512, fat16);
    let clusters = cluster_count(&image);
    assert!((4085..4090).contains(&clusters), "{clusters}");
    exercise(image, 512, FatKind::Fat16, "smallest fat16");

    let fat32 = FormatOptions::new().with_kind(FatKind::Fat32);
    let (sectors, below) = smallest(&fat32, 64 * 1024, 72 * 1024);
    assert_eq!(below, ErrorKind::NoSpace);
    let image = formatted(sectors * 512, 512, fat32);
    let clusters = cluster_count(&image);
    assert!((65525..65530).contains(&clusters), "{clusters}");
    exercise(image, 512, FatKind::Fat32, "smallest fat32");

    let fat12 = FormatOptions::new()
        .with_kind(FatKind::Fat12)
        .with_cluster_size(512);
    let largest = (4100..4200)
        .rev()
        .find(|&sectors| format(device(sectors * 512, 512), fat12.clone()).is_ok())
        .unwrap();
    assert_eq!(
        error((largest + 1) * 512, 512, fat12.clone()),
        ErrorKind::LimitExceeded
    );
    let image = formatted(largest * 512, 512, fat12);
    assert!((4080..=4084).contains(&cluster_count(&image)));
    exercise(
        image,
        512,
        FatKind::Fat12,
        "largest fat12 with 512-byte clusters",
    );
}

fn cluster_count(image: &[u8]) -> u32 {
    let sector = u16::from_le_bytes([image[11], image[12]]) as u32;
    let per_cluster = image[13] as u32;
    let reserved = u16::from_le_bytes([image[14], image[15]]) as u32;
    let fats = image[16] as u32;
    let root = (u16::from_le_bytes([image[17], image[18]]) as u32 * 32).div_ceil(sector);
    let total = match u16::from_le_bytes([image[19], image[20]]) {
        0 => u32::from_le_bytes(image[32..36].try_into().unwrap()),
        small => small as u32,
    };
    let fat = match u16::from_le_bytes([image[22], image[23]]) {
        0 => u32::from_le_bytes(image[36..40].try_into().unwrap()),
        small => small as u32,
    };
    (total - reserved - fats * fat - root) / per_cluster
}

#[test]
fn rejects_bad_options_and_devices() {
    let invalid = [
        FormatOptions::new().with_fat_count(3),
        FormatOptions::new().with_fat_count(0),
        FormatOptions::new().with_media(0x12),
        FormatOptions::new().with_sector_size(768),
        FormatOptions::new().with_cluster_size(1536),
        FormatOptions::new().with_cluster_size(64 * 1024),
        FormatOptions::new().with_root_entries(0),
        FormatOptions::new().with_reserved_sectors(0),
        FormatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_reserved_sectors(4),
    ];
    for options in invalid {
        let mut dev = device(64 * MIB, 512);
        let err = format(&mut dev, options).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(dev.into_inner().iter().all(|&b| b == 0), "nothing written");
    }
    assert_eq!(
        error(
            16 * MIB,
            512,
            FormatOptions::new().with_kind(FatKind::Fat32)
        ),
        ErrorKind::NoSpace
    );
    assert_eq!(
        error(2 * MIB, 512, FormatOptions::new().with_kind(FatKind::Fat16)),
        ErrorKind::NoSpace
    );
    assert_eq!(
        error(
            64 * MIB,
            512,
            FormatOptions::new()
                .with_kind(FatKind::Fat12)
                .with_cluster_size(4096)
        ),
        ErrorKind::LimitExceeded
    );
    let image = vec![0u8; MIB as usize];
    let read_only = MemDevice::new(&image[..], BlockSize::new(512).unwrap());
    assert_eq!(
        format(read_only, FormatOptions::new()).unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
}

#[test]
fn failed_formats_give_the_device_back() {
    let err = format(device(8 * 1024, 512), FormatOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NoSpace);
    assert_eq!(err.into_device().into_inner(), vec![0u8; 8 * 1024]);

    let options = FormatOptions::new().with_kind(FatKind::Fat32);
    let (error, dev) = format(device(4 * MIB, 512), options)
        .unwrap_err()
        .into_parts();
    assert_eq!(error.kind(), ErrorKind::NoSpace);
    assert_eq!(dev.get_ref().len(), (4 * MIB) as usize);

    let image = vec![0xA5u8; MIB as usize];
    let read_only = MemDevice::new(&image[..], BlockSize::new(512).unwrap());
    let err = format(read_only, FormatOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ReadOnly);
    assert!(core::ptr::eq(*err.device().get_ref(), &image[..]));

    let err: hadris_fs::Error<_> = format(device(8 * 1024, 512), FormatOptions::new())
        .map(|_| ())
        .map_err(Into::into)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NoSpace);
}

#[test]
fn labels() {
    assert_eq!(label("boot").as_bytes(), b"BOOT       ");
    assert_eq!(label("My Disk 1").as_str(), "MY DISK 1");
    assert_eq!(label("ABCDEFGHIJK").as_str(), "ABCDEFGHIJK");
    assert_eq!(
        VolumeLabel::new("ABCDEFGHIJKL"),
        Err(ErrorKind::NameTooLong)
    );
    for bad in ["", " LEAD", "A.B", "A*B", "caf\u{E9}", "TAB\t"] {
        assert_eq!(
            VolumeLabel::new(bad),
            Err(ErrorKind::InvalidInput),
            "{bad:?}"
        );
    }
    let image = formatted(2 * MIB, 512, FormatOptions::new());
    assert_eq!(&image[43..54], b"NO NAME    ");
    let root_start = (1 + 2 * u16::from_le_bytes([image[22], image[23]]) as usize) * 512;
    assert!(image[root_start..root_start + 32].iter().all(|&b| b == 0));
}

#[test]
fn formats_a_partition_slice() {
    let mut disk = MemDevice::new(vec![0xAAu8; 8 * MIB as usize], BlockSize::new(512).unwrap());
    let first = 2048;
    let count = 4 * MIB / 512;
    let slice = hadris_storage::Partition::new(&mut disk, first * 512, count * 512);
    let options = FormatOptions::new()
        .with_hidden_sectors(first as u32)
        .with_label(label("PART"));
    let mut fs = format(slice, options).unwrap();
    assert!(fs.stats().unwrap().total_blocks() > 0);
    let _ = fs.into_inner();
    let bytes = disk.into_inner();
    let start = first as usize * 512;
    let end = start + count as usize * 512;
    assert!(bytes[..start].iter().all(|&b| b == 0xAA));
    assert!(bytes[end..].iter().all(|&b| b == 0xAA));
    let part = bytes[start..end].to_vec();
    assert_eq!(u32::from_le_bytes(part[28..32].try_into().unwrap()), 2048);
    exercise(part, 512, FatKind::Fat12, "partition");
}

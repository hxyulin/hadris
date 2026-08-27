use std::io::{Cursor, Read as _, Seek as _, Write as _};

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter, SectorSize};
use hadris_fat::{Error, Fat12, Fat16, Fat32, FatVolume};

#[path = "common/fat.rs"]
mod fat_helpers;
use fat_helpers::{FAT_CASES, FatImage};

fn fat32_header() -> Vec<u8> {
    let mut data = vec![0_u8; 1024];
    data[0..3].copy_from_slice(&[0xeb, 0x58, 0x90]);
    data[3..11].copy_from_slice(b"HADRIS  ");
    data[11..13].copy_from_slice(&512_u16.to_le_bytes());
    data[13] = 1;
    data[14..16].copy_from_slice(&32_u16.to_le_bytes());
    data[16] = 2;
    data[21] = 0xf8;
    data[32..36].copy_from_slice(&100_000_u32.to_le_bytes());
    data[36..40].copy_from_slice(&800_u32.to_le_bytes());
    data[44..48].copy_from_slice(&2_u32.to_le_bytes());
    data[48..50].copy_from_slice(&1_u16.to_le_bytes());
    data[50..52].copy_from_slice(&6_u16.to_le_bytes());
    data[66] = 0x29;
    data[71..82].copy_from_slice(b"TEST       ");
    data[82..90].copy_from_slice(b"FAT32   ");
    data[510..512].copy_from_slice(&0xaa55_u16.to_le_bytes());
    data[512..516].copy_from_slice(&0x4161_5252_u32.to_le_bytes());
    data[996..1000].copy_from_slice(&0x6141_7272_u32.to_le_bytes());
    data[1000..1004].copy_from_slice(&u32::MAX.to_le_bytes());
    data[1004..1008].copy_from_slice(&u32::MAX.to_le_bytes());
    data[1020..1024].copy_from_slice(&0xaa55_0000_u32.to_le_bytes());
    data
}

#[test]
fn bpb_size_validation_uses_production_reader_and_formatter() {
    for sector_size in [
        SectorSize::S512,
        SectorSize::S1024,
        SectorSize::S2048,
        SectorSize::S4096,
    ] {
        let options = FatFormatOptions::new(2 * 1024 * 1024 * 1024)
            .sector_size(sector_size)
            .sectors_per_cluster(4)
            .fat_type(FatTypeSelection::Fat32);
        FatVolumeFormatter::calculate_params(&options).unwrap();
    }

    let mut header = fat32_header();
    header[11..13].copy_from_slice(&768_u16.to_le_bytes());
    assert!(matches!(
        FatVolume::open(Cursor::new(header)),
        Err(Error::CorruptFilesystem { .. })
    ));

    let options = FatFormatOptions::new(64 * 1024 * 1024)
        .sector_size(SectorSize::S4096)
        .sectors_per_cluster(16);
    assert!(matches!(
        FatVolumeFormatter::calculate_params(&options),
        Err(Error::InvalidFormatOption {
            option: "sectors_per_cluster",
            ..
        })
    ));
}

#[test]
fn fat32_rejects_unknown_version_and_invalid_fsinfo_signatures() {
    let mut version = fat32_header();
    version[42..44].copy_from_slice(&1_u16.to_le_bytes());
    assert!(matches!(
        FatVolume::open(Cursor::new(version)),
        Err(Error::CorruptFilesystem { .. })
    ));

    for range in [512..516, 996..1000] {
        let mut image = fat32_header();
        image[range].fill(0);
        assert!(matches!(
            FatVolume::open(Cursor::new(image)),
            Err(Error::InvalidFsInfoSignature { .. })
        ));
    }
}

fn set_fat12(bytes: &mut [u8], cluster: usize, value: u16) {
    let offset = cluster * 3 / 2;
    if cluster.is_multiple_of(2) {
        bytes[offset] = value as u8;
        bytes[offset + 1] = (bytes[offset + 1] & 0xf0) | ((value >> 8) as u8 & 0x0f);
    } else {
        bytes[offset] = (bytes[offset] & 0x0f) | ((value << 4) as u8);
        bytes[offset + 1] = (value >> 4) as u8;
    }
}

#[test]
fn fat_table_traversal_recognizes_bad_and_end_markers_for_every_width() {
    let fat12 = Fat12::new(0, 32, 1, 8);
    let mut bytes = vec![0_u8; 32];
    set_fat12(&mut bytes, 2, 0x0ff7);
    assert!(matches!(
        fat12.next_cluster(&mut Cursor::new(&bytes), 2),
        Err(Error::BadCluster { cluster: 2 })
    ));
    set_fat12(&mut bytes, 2, 0x0ff8);
    assert_eq!(
        fat12.next_cluster(&mut Cursor::new(&bytes), 2).unwrap(),
        None
    );

    let fat16 = Fat16::new(0, 32, 1, 8);
    let mut bytes = vec![0_u8; 32];
    bytes[4..6].copy_from_slice(&0xfff7_u16.to_le_bytes());
    assert!(matches!(
        fat16.next_cluster(&mut Cursor::new(&bytes), 2),
        Err(Error::BadCluster { cluster: 2 })
    ));
    bytes[4..6].copy_from_slice(&0xfff8_u16.to_le_bytes());
    assert_eq!(
        fat16.next_cluster(&mut Cursor::new(&bytes), 2).unwrap(),
        None
    );

    let fat32 = Fat32::new(0, 32, 1, 8);
    let mut bytes = vec![0_u8; 32];
    bytes[8..12].copy_from_slice(&0x0fff_fff7_u32.to_le_bytes());
    assert!(matches!(
        fat32.next_cluster(&mut Cursor::new(&bytes), 2),
        Err(Error::BadCluster { cluster: 2 })
    ));
    bytes[8..12].copy_from_slice(&0x0fff_fff8_u32.to_le_bytes());
    assert_eq!(
        fat32.next_cluster(&mut Cursor::new(&bytes), 2).unwrap(),
        None
    );
}

#[test]
fn directory_iteration_skips_deleted_slots_and_stops_at_end_marker() {
    let image = FatImage::new(FAT_CASES[0]);
    {
        let volume = image.open();
        let root = volume.root_dir();
        for name in ["A.TXT", "B.TXT", "C.TXT", "D.TXT"] {
            volume.create_file(&root, name).unwrap();
        }
        let deleted = root.find("B.TXT").unwrap().unwrap();
        volume.delete(&deleted).unwrap();
    }

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(image.path())
        .unwrap();
    let mut bpb = [0_u8; 512];
    file.read_exact(&mut bpb).unwrap();
    let sector_size = u16::from_le_bytes(bpb[11..13].try_into().unwrap()) as u64;
    let reserved = u16::from_le_bytes(bpb[14..16].try_into().unwrap()) as u64;
    let fat_count = bpb[16] as u64;
    let sectors_per_fat = u16::from_le_bytes(bpb[22..24].try_into().unwrap()) as u64;
    let root_offset = (reserved + fat_count * sectors_per_fat) * sector_size;
    let mut fourth = [0_u8; 32];
    file.seek(std::io::SeekFrom::Start(root_offset + 4 * 32))
        .unwrap();
    file.read_exact(&mut fourth).unwrap();
    file.seek(std::io::SeekFrom::Start(root_offset + 5 * 32))
        .unwrap();
    file.write_all(&fourth).unwrap();
    file.seek(std::io::SeekFrom::Start(root_offset + 4 * 32))
        .unwrap();
    file.write_all(&[0_u8; 32]).unwrap();
    drop(file);

    let volume = image.open();
    let names: Vec<_> = volume
        .root_dir()
        .entries()
        .map(|entry| entry.unwrap().name().into_owned())
        .collect();
    assert_eq!(names, ["A.TXT", "C.TXT"]);
}

#[test]
fn fat32_writes_preserve_each_copy_reserved_high_bits() {
    let mut bytes = vec![0_u8; 32];
    bytes[8..12].copy_from_slice(&0xa000_0000_u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&0xb000_0000_u32.to_le_bytes());
    let fat = Fat32::new(0, 16, 2, 3);
    let mut cursor = Cursor::new(bytes);
    fat.write_clus(&mut cursor, 2, 0x0fff_fff7).unwrap();
    let bytes = cursor.into_inner();
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        0xafff_fff7
    );
    assert_eq!(
        u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        0xbfff_fff7
    );
}

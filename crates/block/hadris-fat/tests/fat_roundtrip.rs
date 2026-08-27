#![cfg(feature = "write")]

use hadris_fat::{FatVolumeReadExt, FatVolumeWriteExt};
use hadris_io::SeekFrom;

#[path = "common/fat.rs"]
mod fat_helpers;
use fat_helpers::{FAT_CASES, FatImage};

fn read_all<DATA>(
    reader: &mut hadris_fat::read::FileReader<'_, DATA>,
) -> hadris_fat::Result<Vec<u8>>
where
    DATA: hadris_fat::Read + hadris_fat::Seek,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

#[test]
fn fat12_fat16_and_fat32_support_core_file_and_directory_operations() {
    let payload: Vec<u8> = (0..32 * 1024).map(|index| (index * 31 + 7) as u8).collect();

    for case in FAT_CASES {
        let image = FatImage::new(case);
        {
            let volume = image.open();
            let root = volume.root_dir();
            let file = volume.create_file(&root, "PAYLOAD.BIN").unwrap();
            let mut writer = volume.write_file(&file).unwrap();
            writer.write(&payload).unwrap();
            writer.finish().unwrap();
        }
        {
            let volume = image.open();
            let root = volume.root_dir();
            let file = root.find("payload.bin").unwrap().unwrap();
            let mut reader = volume.read_file(&file).unwrap();
            assert_eq!(read_all(&mut reader).unwrap(), payload, "{}", case.name);
            volume.create_dir(&root, "SUBDIR").unwrap();
        }
        {
            let volume = image.open();
            let root = volume.root_dir();
            assert!(root.open_dir("subdir").is_ok(), "{}", case.name);
            let file = root.find("PAYLOAD.BIN").unwrap().unwrap();
            volume.truncate(&file, 1234).unwrap();
        }
        {
            let volume = image.open();
            let root = volume.root_dir();
            let file = root.find("PAYLOAD.BIN").unwrap().unwrap();
            let mut reader = volume.read_file(&file).unwrap();
            assert_eq!(
                read_all(&mut reader).unwrap(),
                payload[..1234],
                "{}",
                case.name
            );
            volume.delete(&file).unwrap();
        }
        assert!(
            image
                .open()
                .root_dir()
                .find("PAYLOAD.BIN")
                .unwrap()
                .is_none(),
            "{}",
            case.name
        );
    }
}

#[test]
fn file_reader_seek_crosses_clusters_and_preserves_position_on_error() {
    let image = FatImage::new(FAT_CASES[0]);
    let payload: Vec<u8> = (0..32 * 1024).map(|index| (index * 17 + 3) as u8).collect();
    {
        let volume = image.open();
        let file = volume.create_file(&volume.root_dir(), "SEEK.BIN").unwrap();
        let mut writer = volume.write_file(&file).unwrap();
        writer.write(&payload).unwrap();
        writer.finish().unwrap();
    }

    let volume = image.open();
    let file = volume.root_dir().find("SEEK.BIN").unwrap().unwrap();
    let mut reader = volume
        .read_file(&file)
        .unwrap()
        .with_cached_chain()
        .unwrap();
    assert_eq!(reader.seek(SeekFrom::Start(12_345)).unwrap(), 12_345);
    let mut bytes = [0_u8; 37];
    assert_eq!(reader.read(&mut bytes).unwrap(), bytes.len());
    assert_eq!(bytes, payload[12_345..12_382]);
    assert_eq!(
        reader.seek(SeekFrom::End(-64)).unwrap(),
        payload.len() as u64 - 64
    );
    assert_eq!(reader.seek(SeekFrom::Start(10)).unwrap(), 10);
    assert!(reader.seek(SeekFrom::Current(-11)).is_err());
    assert_eq!(reader.position(), 10);
}

#[test]
fn fat32_directory_growth_and_move_to_root_remain_readable() {
    let image = FatImage::new(FAT_CASES[2]);
    let volume = image.open();
    let root = volume.root_dir();
    let parent = volume.create_dir(&root, "PARENT").unwrap();
    volume.create_dir(&parent, "CHILD").unwrap();
    let child = parent.find("CHILD").unwrap().unwrap();
    volume.rename(&child, &root, "CHILD").unwrap();

    for index in 0..32 {
        volume
            .create_file(&root, &format!("F{index:02}.TXT"))
            .unwrap();
    }
    assert!(root.open_dir("CHILD").is_ok());
    assert!(root.find("F31.TXT").unwrap().is_some());
}

#[test]
fn long_names_with_colliding_short_aliases_remain_distinct() {
    let image = FatImage::new(FAT_CASES[0]);
    {
        let volume = image.open();
        let root = volume.root_dir();
        volume.create_dir(&root, "SOURCE1").unwrap();
        volume.create_dir(&root, "SOURCE2").unwrap();
        let destination = volume.create_dir(&root, "DEST").unwrap();
        let first = root.find("SOURCE1").unwrap().unwrap();
        let second = root.find("SOURCE2").unwrap().unwrap();
        let first = volume
            .rename(&first, &destination, "Renamed Directory 0023")
            .unwrap();
        let second = volume
            .rename(&second, &destination, "Renamed Directory 0028")
            .unwrap();
        assert_ne!(
            first.short_name().raw_bytes(),
            second.short_name().raw_bytes()
        );
    }

    let volume = image.open();
    let destination = volume.root_dir().open_dir("DEST").unwrap();
    assert!(
        destination
            .find("Renamed Directory 0023")
            .unwrap()
            .is_some()
    );
    assert!(
        destination
            .find("Renamed Directory 0028")
            .unwrap()
            .is_some()
    );
}

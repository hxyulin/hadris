//! `format` lays out volumes that `ExFatFs`, `check` and the native tools
//! accept, for every sector size and a range of cluster sizes.

#[path = "common/exfat.rs"]
mod common;
use common::FsPaths;
use hadris_fs::sync::FileSystem;

use common::{Geometry, clean, fsck};
use hadris_fat::exfat::sync::{ExFatFs, format};
use hadris_fat::exfat::{FormatOptions, VolumeLabel};
use hadris_fs::ErrorKind;

#[test]
fn formats_every_sector_size() {
    for sector in [512u32, 1024, 2048, 4096] {
        for block in [512u32, 4096] {
            let dev = common::device(vec![0u8; 16 << 20], block);
            let fs = format(dev, FormatOptions::new().with_sector_size(sector)).unwrap();
            assert_eq!(fs.cluster_size(), 4096);
            let mut dev = fs.into_inner();
            let (_, found) = common::check_dev(&mut dev, 4096);
            assert_eq!(found, [], "{sector}");
            let mut fs = ExFatFs::open(dev).unwrap();
            let root = fs.root();
            let node = common::write_any(&mut fs, root, "a.txt", b"sector");
            fs.forget(node, 1);
            fs.sync().unwrap();
            let image = fs.into_inner().into_inner();
            assert_eq!(1usize << image[108], sector as usize);
            let mut fs = ExFatFs::open(common::device(image.clone(), block)).unwrap();
            assert_eq!(fs.read_to_vec("/A.TXT").unwrap(), b"sector");
            fsck(&image, &format!("{sector}-byte sectors"));
        }
    }
}

#[test]
fn layouts_follow_the_volume_size() {
    for (size, cluster, heap) in [
        (1usize << 20, 4096usize, None),
        (8 << 20, 4096, None),
        (64 << 20, 4096, Some(2 << 20)),
        (300 << 20, 32 << 10, Some(2 << 20)),
    ] {
        let fs = common::formatted(size, FormatOptions::new());
        let image = common::image(fs);
        let geo = Geometry::of(&image);
        assert_eq!(geo.cluster, cluster, "{size}");
        if let Some(heap) = heap {
            assert_eq!(geo.heap, heap, "{size}");
            assert_eq!(geo.fat, 1 << 20, "{size}");
        }
        fsck(&image, &format!("{size} bytes"));
    }
}

#[test]
fn options_are_checked() {
    let dev = || common::device(vec![0u8; 8 << 20], 512);
    for options in [
        FormatOptions::new().with_sector_size(768),
        FormatOptions::new().with_sector_size(8192),
        FormatOptions::new().with_cluster_size(3000),
        FormatOptions::new().with_cluster_size(256),
        FormatOptions::new().with_cluster_size(64 << 20),
        FormatOptions::new().with_alignment(100),
        FormatOptions::new().with_fat_count(0),
        FormatOptions::new().with_fat_count(3),
    ] {
        let err = format(dev(), options).unwrap_err();
        assert_eq!(err.error().kind(), ErrorKind::InvalidInput);
        assert!(
            err.into_device().into_inner().iter().all(|&b| b == 0),
            "nothing is written"
        );
    }
    let tiny = common::device(vec![0u8; 512 << 10], 512);
    assert_eq!(
        format(tiny, FormatOptions::new())
            .unwrap_err()
            .error()
            .kind(),
        ErrorKind::NoSpace
    );
    let big_blocks = common::device(vec![0u8; 8 << 20], 8192);
    assert_eq!(
        format(big_blocks, FormatOptions::new())
            .unwrap_err()
            .error()
            .kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn labels_serials_and_offsets_are_written() {
    let options = FormatOptions::new()
        .with_label(VolumeLabel::new("Données").unwrap())
        .with_volume_id(0xDEAD_BEEF)
        .with_partition_offset(2048);
    let mut fs = common::formatted(8 << 20, options);
    assert_eq!(fs.label_text().unwrap().unwrap(), "Données");
    assert_eq!(fs.volume_id(), 0xDEAD_BEEF);
    let image = common::image(fs);
    assert_eq!(u64::from_le_bytes(image[64..72].try_into().unwrap()), 2048);
    let a = common::image(common::formatted(8 << 20, FormatOptions::new()));
    let b = common::image(common::formatted(8 << 20, FormatOptions::new()));
    assert!(
        a == b,
        "the default clock formats the same bytes every time"
    );
    fsck(&image, "label");
}

#[test]
fn built_image_is_clean() {
    let image = common::build();
    let mut fs = common::mount(&image);
    clean(&mut fs, "build");
    let ran = fsck(&image, "build");
    eprintln!("native checkers run: {ran}");
}

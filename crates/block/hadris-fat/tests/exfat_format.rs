//! `format` lays out volumes that `ExFatFs`, `check` and the native tools
//! accept, for every sector size and a range of cluster sizes.

#[path = "common/exfat.rs"]
mod common;
use common::FsPaths;
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;

use common::{Geometry, clean, fsck};
use hadris_fat::exfat::sync::{ExFatFs, format, write};
use hadris_fat::exfat::{ExFatOptions, VolumeLabel};
use hadris_fs::{Content, ErrorKind, NoClock, Node, Tree};

#[test]
fn formats_every_sector_size() {
    for sector in [512u32, 1024, 2048, 4096] {
        for block in [512u32, 4096] {
            let mut dev = common::device(vec![0u8; 16 << 20], block);
            let geometry = format(&mut dev, &ExFatOptions::new().with_sector_size(sector)).unwrap();
            assert_eq!(1u32 << geometry.cluster_shift(), 4096);
            let (_, found) = common::check_dev(&mut dev, 4096);
            assert_eq!(found, [], "{sector}");
            let mut fs = ExFatFs::mount(dev, MountOptions::new()).unwrap();
            let root = fs.root();
            let node = common::write_any(&mut fs, root, "a.txt", b"sector");
            fs.forget(node, 1);
            fs.sync().unwrap();
            let image = fs.into_inner().into_inner();
            assert_eq!(1usize << image[108], sector as usize);
            let mut fs =
                ExFatFs::mount(common::device(image.clone(), block), MountOptions::new()).unwrap();
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
        let fs = common::formatted(size, ExFatOptions::new());
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
        ExFatOptions::new().with_sector_size(768),
        ExFatOptions::new().with_sector_size(8192),
        ExFatOptions::new().with_cluster_size(3000),
        ExFatOptions::new().with_cluster_size(256),
        ExFatOptions::new().with_cluster_size(64 << 20),
        ExFatOptions::new().with_alignment(100),
        ExFatOptions::new().with_fat_count(0),
        ExFatOptions::new().with_fat_count(3),
    ] {
        let mut dev = dev();
        let err = format(&mut dev, &options).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            dev.into_inner().iter().all(|&b| b == 0),
            "nothing is written"
        );
    }
    let mut tiny = common::device(vec![0u8; 512 << 10], 512);
    assert_eq!(
        format(&mut tiny, &ExFatOptions::new()).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    let mut big_blocks = common::device(vec![0u8; 8 << 20], 8192);
    assert_eq!(
        format(&mut big_blocks, &ExFatOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        format(&mut dev(), &ExFatOptions::new().with_size(16 << 20))
            .unwrap_err()
            .kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(
        format(&mut dev(), &ExFatOptions::new().with_partition_offset(100))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn labels_serials_and_offsets_are_written() {
    let options = ExFatOptions::new()
        .with_label(VolumeLabel::new("Données").unwrap())
        .with_serial(0xDEAD_BEEF)
        .with_partition_offset(2048 * 512);
    let mut fs = common::formatted(8 << 20, options);
    assert_eq!(fs.label_text().unwrap().unwrap(), "Données");
    assert_eq!(fs.volume_id(), 0xDEAD_BEEF);
    let image = common::image(fs);
    assert_eq!(u64::from_le_bytes(image[64..72].try_into().unwrap()), 2048);
    let a = common::image(common::formatted(8 << 20, ExFatOptions::new()));
    let b = common::image(common::formatted(8 << 20, ExFatOptions::new()));
    assert!(a == b, "the default time formats the same bytes every time");
    fsck(&image, "label");

    let mut disk = common::device(vec![0u8; 16 << 20], 512);
    let mut part = hadris_storage::Partition::new(&mut disk, 4 << 20, 8 << 20);
    let geometry = format(&mut part, &ExFatOptions::new().with_seed(1)).unwrap();
    let image = disk.into_inner()[4 << 20..12 << 20].to_vec();
    assert_eq!(
        u64::from_le_bytes(image[64..72].try_into().unwrap()),
        (4 << 20) / 512,
        "the partition offset defaults to the device's"
    );
    assert_ne!(geometry.serial(), common::mount(&a).volume_id());
    fsck(&image, "partition");
}

#[test]
fn writes_a_tree_to_a_growable_device() {
    let mut tree = Tree::new();
    tree.insert("Photos/Été.txt", Node::file(Content::bytes("summer")))
        .unwrap();
    tree.insert("empty.bin", Node::file(Content::empty()))
        .unwrap();
    let options = ExFatOptions::new().with_size(8 << 20);
    let mut image = Vec::new();
    let report = write(&mut image, &tree, &options).unwrap();
    assert_eq!(report.size(), 8 << 20);
    assert_eq!(image.len(), 8 << 20);
    let mut again = Vec::new();
    write(&mut again, &tree, &options).unwrap();
    assert!(image == again, "reproducible");
    let mut fs = common::mount(&image);
    assert_eq!(fs.read_to_vec("/photos/ÉTÉ.TXT").unwrap(), b"summer");
    assert_eq!(
        fs.metadata("/empty.bin").unwrap().modified(),
        Some(NoClock::TIME)
    );
    clean(&mut fs, "write");
    fsck(&image, "write");
}

#[test]
fn built_image_is_clean() {
    let image = common::build();
    let mut fs = common::mount(&image);
    clean(&mut fs, "build");
    let ran = fsck(&image, "build");
    eprintln!("native checkers run: {ran}");
}

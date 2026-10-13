//! `format` lays out volumes that `ExFatFs`, `check` and the native tools
//! accept, for every sector size and a range of cluster sizes.

#[path = "common/advanced.rs"]
mod advanced;
#[path = "common/exfat.rs"]
mod common;
use common::FsPaths;
use hadris_fs::MountOptions;

use common::{Geometry, clean, fsck};
use hadris_fat::exfat::sync::{ExFatFs, format, write};
use hadris_fat::exfat::{ExFatOptions, Guid, VolumeLabel};
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
fn the_default_alignment_follows_the_physical_block_size() {
    let mut dev = advanced::Advanced::new(8 << 20, 512, 64 << 10);
    format(&mut dev, &ExFatOptions::new()).unwrap();
    let image = dev.inner.into_inner();
    let geo = Geometry::of(&image);
    assert_eq!(geo.cluster, 4096);
    assert_eq!((geo.fat % (64 << 10), geo.heap % (64 << 10)), (0, 0));
    fsck(&image, "64 KiB physical blocks");

    let mut dev = advanced::Advanced::new(8 << 20, 512, 64 << 10);
    format(&mut dev, &ExFatOptions::new().with_alignment(4096)).unwrap();
    let geo = Geometry::of(&dev.inner.into_inner());
    assert_eq!(geo.fat, 24 * 512, "an explicit alignment wins");
}

#[test]
fn the_cluster_count_is_the_largest_that_fits() {
    use common::{le32, le64};

    let sizes = (1u64 << 20..8 << 20).step_by(64 << 10 | 512);
    for size in sizes.chain([4 << 20, 100 << 20]) {
        let mut dev = common::device(vec![0u8; size as usize], 512);
        format(&mut dev, &ExFatOptions::new()).unwrap();
        let image = dev.into_inner();
        let sector = 1u64 << image[108];
        let per_cluster = image[109];
        let volume = le64(&image, 72);
        let fat_offset = le32(&image, 80) as u64;
        let fat_length = le32(&image, 84) as u64;
        let heap = le32(&image, 88) as u64;
        let count = le32(&image, 92) as u64;
        let fats = image[110] as u64;
        let align = if size >= 64 << 20 { 1 << 20 } else { 4096 } / sector;
        assert!(fat_length >= ((count + 2) * 4).div_ceil(sector), "{size}");
        assert!(heap >= fat_offset + fat_length * fats, "{size}");
        assert_eq!(heap % align, 0, "{size}");
        assert!(count <= (volume - heap) >> per_cluster, "{size}");
        let bigger = ((count + 3) * 4).div_ceil(sector);
        let bigger_heap = (fat_offset + bigger * fats).next_multiple_of(align);
        assert!(
            (volume - bigger_heap) >> per_cluster < count + 1,
            "{size}: {} clusters fit",
            count + 1
        );
        if size == 4 << 20 {
            assert_eq!(count, 1020);
            fsck(&image, "4 MiB");
        }
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
        .with_label(VolumeLabel::try_from("Données").unwrap())
        .with_serial(0xDEAD_BEEF)
        .with_partition_offset(2048 * 512);
    let mut fs = common::formatted(8 << 20, options);
    assert_eq!(fs.label_text().unwrap().unwrap(), "Données");
    assert_eq!(fs.info().volume_serial(), 0xDEAD_BEEF);
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
    assert_ne!(
        geometry.volume_serial(),
        common::mount(&a).info().volume_serial()
    );
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
        Some(NoClock::TIME.with_utc_offset_minutes(Some(0)).unwrap()),
        "times without a zone are stored as UTC with a valid offset"
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

/// The `SetChecksum` of a one-entry set, computed as the specification
/// gives it.
fn guid_checksum(entry: &[u8]) -> u16 {
    let mut sum = 0u16;
    for (at, &byte) in entry.iter().enumerate() {
        if at != 2 && at != 3 {
            sum = (if sum & 1 != 0 { 0x8000u16 } else { 0 })
                .wrapping_add(sum >> 1)
                .wrapping_add(u16::from(byte));
        }
    }
    sum
}

#[test]
fn volume_guids_derive_from_the_seed_and_follow_the_specification() {
    let guid = |options: ExFatOptions| common::formatted(8 << 20, options).volume_guid().unwrap();
    let seeded = guid(ExFatOptions::new().with_seed(7)).unwrap();
    assert_eq!(guid(ExFatOptions::new().with_seed(7)), Some(seeded));
    assert_ne!(guid(ExFatOptions::new().with_seed(8)), Some(seeded));
    assert_ne!(guid(ExFatOptions::new()), Some(seeded));
    assert_eq!(seeded.to_bytes()[7] >> 4, 4);
    let fixed: Guid = "0123abcd-4567-89ef-0123-456789abcdef".parse().unwrap();
    assert_eq!(
        guid(ExFatOptions::new().with_volume_guid(Some(fixed))),
        Some(fixed)
    );
    assert_eq!(guid(ExFatOptions::new().with_volume_guid(None)), None);
    let mut dev = common::device(vec![0u8; 8 << 20], 512);
    let nil = ExFatOptions::new().with_volume_guid(Some(Guid::NIL));
    assert_eq!(
        format(&mut dev, &nil).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );

    let options = ExFatOptions::new()
        .with_label(VolumeLabel::new("GUID").unwrap())
        .with_volume_guid(Some(fixed));
    let image = common::image(common::formatted(8 << 20, options));
    let geo = Geometry::of(&image);
    let entries = geo.root_entries(&image, 0xA0);
    assert_eq!(entries.len(), 1);
    let entry = &image[entries[0]..entries[0] + 32];
    assert_eq!(entry[1], 0, "SecondaryCount");
    assert_eq!(&entry[4..6], [0, 0], "GeneralPrimaryFlags");
    assert_eq!(
        &entry[6..22],
        [
            0xcd, 0xab, 0x23, 0x01, 0x67, 0x45, 0xef, 0x89, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef
        ]
    );
    assert_eq!(entry[22..], [0; 10]);
    assert_eq!(
        u16::from_le_bytes([entry[2], entry[3]]),
        guid_checksum(entry)
    );
    fsck(&image, "volume guid");

    let mut tree = Tree::new();
    tree.insert("a.txt", Node::file(Content::bytes("a")))
        .unwrap();
    let options = ExFatOptions::new().with_size(8 << 20).with_seed(1);
    let mut a = Vec::new();
    write(&mut a, &tree, &options).unwrap();
    tree.insert("b.txt", Node::file(Content::bytes("b")))
        .unwrap();
    let mut b = Vec::new();
    write(&mut b, &tree, &options).unwrap();
    assert_ne!(
        common::mount(&a).volume_guid().unwrap(),
        common::mount(&b).volume_guid().unwrap(),
        "different trees get different GUIDs"
    );
}

#[test]
fn volume_guids_are_set_replaced_and_removed() {
    let first: Guid = "11111111-2222-3333-4444-555555555555".parse().unwrap();
    let second: Guid = "66666666-7777-8888-9999-aaaaaaaaaaaa".parse().unwrap();
    let mut fs = common::formatted(8 << 20, ExFatOptions::new().with_volume_guid(None));
    assert_eq!(fs.volume_guid().unwrap(), None);
    fs.set_volume_guid(Some(first)).unwrap();
    assert_eq!(fs.volume_guid().unwrap(), Some(first));
    fs.set_volume_guid(Some(second)).unwrap();
    assert_eq!(
        fs.set_volume_guid(Some(Guid::NIL)).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    fs.sync().unwrap();
    let image = common::image(fs);
    assert_eq!(Geometry::of(&image).root_entries(&image, 0xA0).len(), 1);
    let mut fs = common::mount(&image);
    assert_eq!(fs.volume_guid().unwrap(), Some(second));
    clean(&mut fs, "set guid");
    fsck(&image, "set guid");

    fs.set_volume_guid(None).unwrap();
    assert_eq!(fs.volume_guid().unwrap(), None);
    fs.set_volume_guid(None).unwrap();
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    assert_eq!(geo.root_entries(&image, 0xA0), []);
    assert_eq!(geo.root_entries(&image, 0x20).len(), 1);
    fsck(&image, "removed guid");

    let mut corrupt = common::image(common::formatted(
        8 << 20,
        ExFatOptions::new().with_volume_guid(Some(first)),
    ));
    let at = Geometry::of(&corrupt).root_entries(&corrupt, 0xA0)[0];
    corrupt[at + 10] ^= 1;
    let mut fs = common::mount(&corrupt);
    assert_eq!(fs.volume_guid().unwrap(), None, "a bad SetChecksum");
    fs.set_volume_guid(Some(first)).unwrap();
    assert_eq!(fs.volume_guid().unwrap(), Some(first));
}

#[test]
fn volume_guids_match_tune_exfat() {
    let guid: Guid = "0123abcd-4567-89ef-0123-456789abcdef".parse().unwrap();
    let image = common::image(common::formatted(
        8 << 20,
        ExFatOptions::new().with_volume_guid(Some(guid)),
    ));
    let Some((printed, _)) = common::tune_exfat(&image, &["-u"]) else {
        if std::env::var_os("HADRIS_REQUIRE_EXTERNAL_TOOLS").is_some() {
            panic!("tune.exfat is unavailable");
        }
        eprintln!("skipping: tune.exfat is not available");
        return;
    };
    assert!(
        printed.to_lowercase().contains(&guid.to_string()),
        "tune.exfat -u printed {printed:?}"
    );

    let other: Guid = "fedcba98-7654-3210-fedc-ba9876543210".parse().unwrap();
    let (_, tuned) = common::tune_exfat(&image, &["-U", &other.to_string()]).unwrap();
    let mut fs = common::mount(&tuned);
    assert_eq!(fs.volume_guid().unwrap(), Some(other));
    clean(&mut fs, "tune.exfat -U");

    let image = common::image(common::formatted(8 << 20, ExFatOptions::new().with_seed(3)));
    let expected = common::mount(&image).volume_guid().unwrap().unwrap();
    let (printed, _) = common::tune_exfat(&image, &["-u"]).unwrap();
    assert!(printed.to_lowercase().contains(&expected.to_string()));
}

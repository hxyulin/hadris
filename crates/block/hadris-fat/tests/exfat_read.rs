//! `ExFatFs` reads volumes laid out in ways its own writer does not
//! produce: other tools' images, contiguous allocations, entry sets across
//! clusters, fragmented system structures and short valid data lengths.

#[path = "common/exfat.rs"]
mod common;

use common::{Geometry, Tool, clean, fsck, fsck_with, le32, put32};
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::{DirCursor, ErrorKind, FileType, NameBuf};

type Patch = Box<dyn Fn(&mut Vec<u8>)>;
type Damage = fn(&mut [u8]);

fn mount_err(image: Vec<u8>) -> ErrorKind {
    let err = ExFatFs::open(common::device(image, 512)).unwrap_err();
    err.error().kind()
}

#[test]
fn mount_rejects_bad_boot_sectors() {
    let base = common::image(common::small(4 << 20, 4096));
    let geo = Geometry::of(&base);
    let patches: [(&str, Patch); 11] = [
        ("name", Box::new(|i| i[3] = b'F')),
        ("signature", Box::new(|i| i[510] = 0)),
        ("must be zero", Box::new(|i| i[20] = 1)),
        ("sector shift", Box::new(|i| i[108] = 8)),
        ("cluster shift", Box::new(|i| i[109] = 20)),
        ("three FATs", Box::new(|i| i[110] = 3)),
        ("revision", Box::new(|i| i[105] = 2)),
        ("no clusters", Box::new(|i| put32(i, 92, 0))),
        ("root", Box::new(|i| put32(i, 96, 1))),
        (
            "volume length",
            Box::new(|i| i[72..80].copy_from_slice(&(1u64 << 40).to_le_bytes())),
        ),
        (
            "no bitmap",
            Box::new(move |i| {
                let at = geo.root_entries(i, 0x81)[0];
                i[at] = 0x01;
            }),
        ),
    ];
    for (what, patch) in patches {
        let mut image = base.clone();
        patch(&mut image);
        common::seal_boot(&mut image, 512);
        let want = match what {
            "name" => ErrorKind::NotRecognized,
            _ => ErrorKind::Corrupt,
        };
        assert_eq!(mount_err(image), want, "{what}");
    }
    let mut image = base.clone();
    let at = geo.root_entries(&image, 0x82)[0];
    put32(&mut image, at + 20, 0);
    assert_eq!(mount_err(image), ErrorKind::Corrupt, "up-case table");
    let err = ExFatFs::open(common::device(vec![0u8; 1 << 20], 512)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotRecognized);
    assert_eq!(
        err.into_device().into_inner().len(),
        1 << 20,
        "the device comes back"
    );
}

#[test]
fn damaged_main_boot_regions_mount_from_the_backup() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "kept.txt", b"backup");
    fs.forget(node);
    fs.sync().unwrap();
    let base = common::image(fs);
    let damages: [(&str, Damage); 3] = [
        ("checksum", |i| i[11 * 512] ^= 1),
        ("boot code", |i| i[200] ^= 1),
        ("name", |i| i[3] = b'F'),
    ];
    for (what, damage) in damages {
        let mut image = base.clone();
        damage(&mut image);
        let mut fs = ExFatFs::open(common::device(image.clone(), 512)).unwrap();
        assert!(fs.is_read_only(), "{what}");
        assert_eq!(fs.read_to_vec("/kept.txt").unwrap(), b"backup", "{what}");
        let root = fs.root();
        let created = fs.create(
            root,
            hadris_fs::Name::new("new.txt").unwrap(),
            hadris_fs::NewNode::File,
            &hadris_fs::SetMetadata::new(),
        );
        assert_eq!(created.unwrap_err().kind(), ErrorKind::ReadOnly, "{what}");
        damage(&mut image[12 * 512..]);
        let want = match what {
            "name" => ErrorKind::NotRecognized,
            _ => ErrorKind::Corrupt,
        };
        assert_eq!(mount_err(image), want, "{what} in both");
    }
}

#[test]
fn reads_a_macos_image() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/exfat_partition.img"
    ))
    .unwrap();
    let mut fs = common::mount(&image);
    assert_eq!(fs.label().unwrap().unwrap().to_string(), "TESTEXFAT");
    assert_eq!(fs.read_to_vec("/hello.txt").unwrap(), b"Hello, exFAT\\!\n");
    assert_eq!(
        fs.read_to_vec("/SUBDIR/Nested.TXT").unwrap(),
        b"Nested file\n"
    );
    let names = common::names(&mut fs, "/");
    assert!(
        names.contains(&"hello.txt".to_owned()) && names.contains(&"subdir".to_owned()),
        "{names:?}"
    );
    let report = hadris_fat::exfat::sync::check(&mut fs).unwrap();
    assert!(report.files() >= 2 && report.directories() >= 2);
    let root = fs.root();
    let node = common::write(&mut fs, root, "added.txt", b"from hadris");
    fs.forget(node);
    fs.remove(
        root,
        hadris_fs::Name::new("hello.txt").unwrap(),
        hadris_fs::RemoveKind::File,
    )
    .unwrap();
    fs.sync().unwrap();
    clean(&mut fs, "macOS image");
    fsck(&common::image(fs), "macOS image");
}

#[test]
fn contiguous_files_are_read() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    let data = common::payload(10_000, 7);
    let node = common::write(&mut fs, root, "contig.bin", &data);
    fs.forget(node);
    let dir = common::mkdir(&mut fs, root, "cdir");
    for i in 0..40 {
        let node = common::write(&mut fs, dir, &format!("entry {i:02}"), b"");
        fs.forget(node);
    }
    fs.forget(dir);
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "contig.bin");
    geo.unchain(&mut image, &set);
    let set = geo.set(&image, geo.root, "cdir");
    geo.unchain(&mut image, &set);
    let mut fs = common::mount(&image);
    assert_eq!(fs.read_to_vec("/contig.bin").unwrap(), data);
    assert_eq!(common::names(&mut fs, "/cdir").len(), 40);
    assert_eq!(fs.read_to_vec("/cdir/ENTRY 39").unwrap(), b"");
    clean(&mut fs, "contiguous");
    fsck(&image, "contiguous");
}

#[test]
fn entry_sets_cross_clusters() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    let dir = common::mkdir(&mut fs, root, "sets");
    let mut names = Vec::new();
    for i in 0..60 {
        let name = format!("a name that needs three entries {i:02}");
        let node = common::write(&mut fs, dir, &name, name.as_bytes());
        fs.forget(node);
        names.push(name);
        if i % 7 == 3 {
            let spacer = common::write(&mut fs, root, &format!("spacer {i}"), &[1; 600]);
            fs.forget(spacer);
        }
    }
    fs.forget(dir);
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "sets");
    let slots = geo.slots(&image, le32(&image, set[1] + 20));
    let mut far = 0;
    for (index, &at) in slots.iter().enumerate() {
        if image[at] == 0x85 {
            let count = 1 + image[at + 1] as usize;
            if slots[index..index + count]
                .windows(2)
                .any(|pair| pair[1] != pair[0] + 32)
            {
                far += 1;
            }
        }
    }
    assert!(
        far > 0,
        "some set must cross into a cluster that does not follow"
    );

    let mut fs = common::mount(&image);
    assert_eq!(common::names(&mut fs, "/sets"), names);
    let dir = fs.resolve("/sets").unwrap();
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, &mut buf).unwrap() {
        let meta = fs.node_metadata(entry.node()).unwrap();
        assert_eq!(meta.file_type(), FileType::File);
        assert_eq!(meta.len(), buf.len() as u64);
        let mut data = vec![0u8; buf.len()];
        fs.read_at(entry.node(), 0, &mut data).unwrap();
        assert_eq!(data, buf.as_bytes());
    }
    for name in &names {
        let upper = name.to_uppercase();
        assert_eq!(
            fs.read_to_vec(&format!("/sets/{upper}")).unwrap(),
            name.as_bytes()
        );
    }
    let node = fs.resolve(&format!("/sets/{}", names[40])).unwrap();
    let parent = fs.parent(node);
    assert!(parent.is_err(), "a file has no parent directory to report");
    fs.forget(node);
    fs.forget(dir);
    clean(&mut fs, "crossing sets");
    fsck(&image, "crossing sets");
}

#[test]
fn valid_data_length_reads_zeros() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "vdl.bin", &[0xAA; 1000]);
    fs.set_len(node, 10_000).unwrap();
    let mut buf = vec![0xFFu8; 10_000];
    assert_eq!(fs.read_at(node, 0, &mut buf).unwrap(), 10_000);
    assert!(buf[..1000].iter().all(|&b| b == 0xAA) && buf[1000..].iter().all(|&b| b == 0));
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "vdl.bin");
    let stream = set[1];
    assert_eq!(
        u64::from_le_bytes(image[stream + 8..stream + 16].try_into().unwrap()),
        1000
    );
    assert_eq!(
        u64::from_le_bytes(image[stream + 24..stream + 32].try_into().unwrap()),
        10_000
    );
    fsck(&image, "short valid data length");

    let mut fs = common::mount(&image);
    let node = fs.resolve("/vdl.bin").unwrap();
    fs.write_at(node, 5000, b"x").unwrap();
    fs.sync().unwrap();
    let data = fs.read_to_vec("/vdl.bin").unwrap();
    assert_eq!(data.len(), 10_000);
    assert!(data[1000..5000].iter().all(|&b| b == 0));
    assert_eq!(data[5000], b'x');
    fs.forget(node);
    let mut image = common::image(fs);
    let set = geo.set(&image, geo.root, "vdl.bin");
    assert_eq!(
        u64::from_le_bytes(image[set[1] + 8..set[1] + 16].try_into().unwrap()),
        5001
    );

    image[set[1] + 8..set[1] + 16].copy_from_slice(&100u64.to_le_bytes());
    geo.reseal(&mut image, &set);
    let mut fs = common::mount(&image);
    let data = fs.read_to_vec("/vdl.bin").unwrap();
    assert!(data[..100].iter().all(|&b| b == 0xAA) && data[100..].iter().all(|&b| b == 0));
}

#[test]
fn fragmented_bitmap_and_upcase_table() {
    let mut image = common::image(common::small(8 << 20, 512));
    let geo = Geometry::of(&image);
    let bitmap = le32(&image, geo.root_entries(&image, 0x81)[0] + 20);
    let upcase = le32(&image, geo.root_entries(&image, 0x82)[0] + 20);
    assert_eq!(geo.chain(&image, bitmap).len(), 4);
    geo.relocate(&mut image, bitmap, 2, geo.count - 10);
    geo.relocate(&mut image, upcase, 5, geo.count - 20);
    geo.relocate(&mut image, upcase, 11, geo.count - 30);
    // exfatprogs 1.2.9 reads the bitmap and up-case table as if they were
    // contiguous, so only macOS `fsck_exfat` can check these images.
    fsck_with(&image, "fragmented system structures", &[Tool::MacOs]);

    let mut fs = common::mount(&image);
    clean(&mut fs, "fragmented system structures");
    let root = fs.root();
    let greek = "\u{3B1}\u{3B2}\u{3B3} \u{430}\u{431}\u{432} \u{FF41}.txt";
    let node = common::write(&mut fs, root, greek, b"greek");
    fs.forget(node);
    let upper = "\u{391}\u{392}\u{393} \u{410}\u{411}\u{412} \u{FF21}.TXT";
    assert_eq!(fs.read_to_vec(&format!("/{upper}")).unwrap(), b"greek");
    let big = common::payload(5 << 20, 9);
    let node = common::write(&mut fs, root, "big.bin", &big);
    fs.forget(node);
    fs.sync().unwrap();
    assert_eq!(fs.read_to_vec("/big.bin").unwrap(), big);
    clean(&mut fs, "filled");
    let image = common::image(fs);
    assert!(
        Geometry::of(&image)
            .chain(&image, bitmap)
            .contains(&(geo.count - 10))
    );
    fsck_with(
        &image,
        "filled past the moved bitmap cluster",
        &[Tool::MacOs],
    );
}

#[test]
fn cyclic_chains_are_corrupt_instead_of_repeating() {
    let image = common::build();
    let geo = Geometry::of(&image);
    let (inner, deep) = {
        let mut fs = common::mount(&image);
        let inner = fs.resolve("/Nested Dir/inner").unwrap();
        let deep = fs.resolve("/Nested Dir/inner/deep.bin").unwrap();
        (common::chain(&mut fs, inner), common::chain(&mut fs, deep))
    };
    assert!(inner.len() > 2 && deep.len() > 4);

    let mut dir_image = image.clone();
    geo.set_fat(&mut dir_image, inner[1], inner[0]);
    let mut fs = common::mount(&dir_image);
    let dir = fs.resolve("/Nested Dir/inner").unwrap();
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let listed = loop {
        match fs.read_dir_entry(dir, &mut cursor, &mut buf) {
            Ok(Some(_)) => {}
            Ok(None) => break Ok(()),
            Err(err) => break Err(err.kind()),
        }
    };
    assert_eq!(listed, Err(ErrorKind::Corrupt));
    hadris_fat::exfat::sync::check(&mut fs).unwrap();

    let mut file_image = image.clone();
    geo.set_fat(&mut file_image, deep[3], deep[1]);
    let mut fs = common::mount(&file_image);
    let file = fs.resolve("/Nested Dir/inner/deep.bin").unwrap();
    let mut chunk = [0u8; 777];
    let mut at = 0;
    let read = loop {
        match fs.read_at(file, at, &mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => at += n as u64,
            Err(err) => break Err(err.kind()),
        }
    };
    assert_eq!(read, Err(ErrorKind::Corrupt));
    let mut all = vec![0u8; 70_000];
    assert_eq!(
        fs.read_at(file, 0, &mut all).map_err(|err| err.kind()),
        Err(ErrorKind::Corrupt)
    );
}

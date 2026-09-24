//! `check` finds damage on exFAT volumes and reports the same findings with
//! any bitmap size, without changing the volume.

#[path = "common/exfat.rs"]
mod common;

use common::{Found, Geometry, le32, put32};
use hadris_fat::exfat::Detail;
use hadris_fat::exfat::sync::check;
use hadris_fs::{CheckReport, ErrorKind, Location, Severity};

/// Checks `image` with a cluster bitmap of `bitmap` bytes.
fn run(image: &[u8], bitmap: usize) -> (CheckReport, Vec<Found>) {
    let mut dev = common::device(image.to_vec(), 512);
    let found = common::check_dev(&mut dev, 1024 + bitmap);
    assert_eq!(dev.into_inner(), image, "check changes nothing");
    found
}

fn sorted(found: &[Found]) -> Vec<String> {
    let mut all: Vec<_> = found.iter().map(|found| format!("{found:?}")).collect();
    all.sort();
    all
}

fn kinds(image: &[u8]) -> Vec<Detail> {
    let (_, findings) = run(image, 4096);
    let (_, small) = run(image, 512);
    assert_eq!(
        sorted(&findings),
        sorted(&small),
        "findings do not depend on the bitmap size"
    );
    let mut kinds: Vec<_> = findings.iter().map(|found| found.detail).collect();
    kinds.sort_by_key(|kind| *kind as u32);
    kinds
}

fn base() -> Vec<u8> {
    common::build()
}

#[test]
fn clean_volumes_report_their_contents() {
    let image = base();
    let (report, findings) = run(&image, 4096);
    assert!(report.is_clean(), "{findings:?}");
    assert_eq!(report.passes(), 1);
    let mut dev = common::device(image, 512);
    let err = check(&mut dev, &mut [0u8; 1535], |_| {}).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn findings_do_not_depend_on_the_window() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    for i in 0..40 {
        let node = common::write(
            &mut fs,
            root,
            &format!("f{i}"),
            &common::payload(3000, i as u8),
        );
        fs.forget(node);
    }
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    let (report, found) = run(&image, 512);
    assert_eq!(found, []);
    assert!(report.passes() > 1, "{report:?}");
    let mut damaged = image.clone();
    geo.set_bit(&mut damaged, geo.count - 3, true);
    let set = geo.set(&damaged, geo.root, "f7");
    let first = le32(&damaged, set[1] + 20);
    let chain = geo.chain(&damaged, first);
    geo.set_fat(&mut damaged, chain[1], chain[0]);
    assert_eq!(
        kinds(&damaged),
        [
            Detail::CyclicChain,
            Detail::LostClusters,
            Detail::LostClusters
        ]
    );
}

#[test]
fn bitmap_mismatches_are_found() {
    let image = base();
    let geo = Geometry::of(&image);
    let mut lost = image.clone();
    geo.set_bit(&mut lost, geo.count - 5, true);
    geo.set_bit(&mut lost, geo.count - 4, true);
    assert_eq!(kinds(&lost), [Detail::LostClusters]);
    let (report, findings) = run(&lost, 4096);
    assert_eq!(report.findings(), 1);
    assert_eq!(
        findings[0].location,
        Some(Location::Cluster(geo.count as u64 - 5)),
        "{findings:?}"
    );

    let mut bad = image.clone();
    geo.set_bit(&mut bad, geo.count - 5, true);
    geo.set_fat(&mut bad, geo.count - 5, 0xFFFF_FFF7);
    let (_, findings) = run(&bad, 4096);
    assert!(findings.is_empty(), "{findings:?}");

    let mut freed = image.clone();
    let set = geo.set(&freed, geo.root, "README.TXT");
    let first = le32(&freed, set[1] + 20);
    geo.set_bit(&mut freed, first, false);
    assert_eq!(kinds(&freed), [Detail::Bitmap]);
}

#[test]
fn upcase_checksum_is_checked() {
    let mut image = base();
    let geo = Geometry::of(&image);
    let at = geo.root_entries(&image, 0x82)[0];
    put32(&mut image, at + 4, 0x1234_5678);
    assert!(common::mount(&image).is_read_only());
    assert_eq!(kinds(&image), [Detail::UpcaseTable]);
}

#[test]
fn boot_region_damage_is_found() {
    let image = base();
    let mut checksum = image.clone();
    checksum[11 * 512] ^= 1;
    assert!(kinds(&checksum).contains(&Detail::BootChecksum));
    let mut backup = image.clone();
    backup[12 * 512 + 100] ^= 1;
    assert_eq!(kinds(&backup), [Detail::BackupBootRegion]);
    let mut extended = image.clone();
    extended[2 * 512 - 1] = 0;
    assert!(kinds(&extended).contains(&Detail::BootSector));
    let mut dirty = image.clone();
    dirty[106] |= 2;
    assert_eq!(kinds(&dirty), [Detail::Dirty]);
    let (_, found) = run(&dirty, 4096);
    assert_eq!(found[0].severity, Severity::Notice);
    assert_eq!(found[0].location, Some(Location::Byte(106)));
    let mut percent = image.clone();
    percent[112] = 77;
    assert_eq!(kinds(&percent), [Detail::PercentInUse]);
    let mut unknown = image.clone();
    unknown[112] = 0xFF;
    assert!(kinds(&unknown).is_empty());
    let mut fat = image.clone();
    let geo = Geometry::of(&fat);
    geo.set_fat(&mut fat, 0, 0);
    assert_eq!(kinds(&fat), [Detail::FatEntries]);
}

#[test]
fn entry_set_damage_is_found() {
    let image = base();
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "lower.txt");

    let mut sum = image.clone();
    sum[set[0] + 2] ^= 1;
    assert_eq!(kinds(&sum), [Detail::SetChecksum]);
    let (_, found) = run(&sum, 4096);
    assert_eq!(found[0].path.as_deref(), Some("/lower.txt"));
    assert_eq!(found[0].location, Some(Location::Byte(set[0] as u64)));

    let mut hash = image.clone();
    hash[set[1] + 4] ^= 1;
    geo.reseal(&mut hash, &set);
    assert_eq!(kinds(&hash), [Detail::NameHash]);

    let mut bad_name = image.clone();
    bad_name[set[2] + 2] = b'*';
    geo.reseal(&mut bad_name, &set);
    let found = kinds(&bad_name);
    assert!(found.contains(&Detail::BadName), "{found:?}");

    let mut vdl = image.clone();
    vdl[set[1] + 8..set[1] + 16].copy_from_slice(&1000u64.to_le_bytes());
    geo.reseal(&mut vdl, &set);
    assert_eq!(kinds(&vdl), [Detail::ValidDataLength]);

    let mut broken = image.clone();
    broken[set[0] + 1] = 5;
    let found = kinds(&broken);
    assert!(found.contains(&Detail::EntrySet), "{found:?}");

    let dir = geo.set(&image, geo.root, "Nested Dir");
    let mut size = image.clone();
    size[dir[1] + 24] = 1;
    size[dir[1] + 8] = 1;
    geo.reseal(&mut size, &dir);
    assert!(kinds(&size).contains(&Detail::DirectorySize));

    let mut label = image.clone();
    let at = geo.root_entries(&label, 0x83)[0];
    label[at + 1] = 12;
    assert_eq!(kinds(&label), [Detail::RootEntry]);
}

#[test]
fn chain_damage_is_found() {
    let image = base();
    let geo = Geometry::of(&image);
    let frag = geo.set(&image, geo.root, "frag.bin");
    let first = le32(&image, frag[1] + 20);
    let chain = geo.chain(&image, first);

    let mut cyclic = image.clone();
    geo.set_fat(&mut cyclic, *chain.last().unwrap(), chain[1]);
    let found = kinds(&cyclic);
    assert!(found.contains(&Detail::CyclicChain), "{found:?}");

    let mut broken = image.clone();
    geo.set_fat(&mut broken, chain[2], 0);
    let found = kinds(&broken);
    assert!(
        found.contains(&Detail::BrokenChain) && found.contains(&Detail::LostClusters),
        "{found:?}"
    );

    let mut short = image.clone();
    geo.set_fat(&mut short, chain[2], 0xFFFF_FFFF);
    let found = kinds(&short);
    assert!(found.contains(&Detail::SizeMismatch), "{found:?}");

    let mut bad = image.clone();
    geo.set_fat(&mut bad, chain[2], 0xFFFF_FFF7);
    let found = kinds(&bad);
    assert!(found.contains(&Detail::BadCluster), "{found:?}");

    let readme = geo.set(&image, geo.root, "README.TXT");
    let mut cross = image.clone();
    put32(&mut cross, readme[1] + 20, chain[0]);
    geo.reseal(&mut cross, &readme);
    let found = kinds(&cross);
    assert!(found.contains(&Detail::CrossLink), "{found:?}");

    let mut invalid = image.clone();
    put32(&mut invalid, readme[1] + 20, geo.count + 2);
    geo.reseal(&mut invalid, &readme);
    let found = kinds(&invalid);
    assert!(found.contains(&Detail::InvalidCluster), "{found:?}");

    let mut long = image.clone();
    let lower = geo.set(&long, geo.root, "lower.txt");
    long[lower[1] + 24..lower[1] + 32].copy_from_slice(&0u64.to_le_bytes());
    long[lower[1] + 8..lower[1] + 16].copy_from_slice(&0u64.to_le_bytes());
    geo.reseal(&mut long, &lower);
    let found = kinds(&long);
    assert!(found.contains(&Detail::SizeMismatch), "{found:?}");
}

#[test]
fn deep_trees_are_reported() {
    let mut fs = common::small(8 << 20, 512);
    let mut dir = fs.root();
    for i in 0..66 {
        let next = common::mkdir(&mut fs, dir, &format!("d{i}"));
        if dir != fs.root() {
            fs.forget(dir);
        }
        dir = next;
    }
    let node = common::write(&mut fs, dir, "bottom", b"deep");
    fs.forget(node);
    fs.forget(dir);
    fs.sync().unwrap();
    let image = common::image(fs);
    let found = kinds(&image);
    assert!(found.contains(&Detail::TooDeep), "{found:?}");
}

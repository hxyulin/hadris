//! `check` finds damage on exFAT volumes and reports the same findings with
//! any bitmap size, without changing the volume.

#[path = "common/exfat.rs"]
mod common;

use common::{Geometry, le32, put32};
use hadris_fat::exfat::sync::check_with;
use hadris_fat::exfat::{CheckReport, Finding, FindingKind};

fn run(image: &[u8], bitmap: usize) -> (CheckReport, Vec<Finding>) {
    let mut fs = common::mount(image);
    let mut findings = Vec::new();
    let report = check_with(&mut fs, &mut vec![0u8; bitmap], |f| findings.push(f)).unwrap();
    assert_eq!(common::image(fs), image, "check changes nothing");
    (report, findings)
}

fn kinds(image: &[u8]) -> Vec<FindingKind> {
    let (_, findings) = run(image, 4096);
    let (_, small) = run(image, 1);
    let mut a: Vec<_> = findings.iter().map(Finding::kind).collect();
    let mut b: Vec<_> = small.iter().map(Finding::kind).collect();
    a.sort_by_key(|k| *k as u32);
    b.sort_by_key(|k| *k as u32);
    assert_eq!(a, b, "findings do not depend on the bitmap size");
    a
}

fn base() -> Vec<u8> {
    common::build()
}

#[test]
fn clean_volumes_report_their_contents() {
    let image = base();
    let (report, findings) = run(&image, 4096);
    assert!(report.is_clean(), "{findings:?}");
    assert_eq!(report.files(), 5 + 2 + 300 + 1);
    assert_eq!(report.directories(), 3);
    assert_eq!(report.passes(), 1);
    let (small, _) = run(&image, 1);
    assert!(small.passes() > 1);
    assert_eq!(small.used_clusters(), report.used_clusters());
    assert_eq!(small.free_clusters(), report.free_clusters());
    assert!(
        hadris_fat::exfat::sync::check_with(&mut common::mount(&image), &mut [], |_| {}).is_err()
    );
}

#[test]
fn bitmap_mismatches_are_found() {
    let image = base();
    let geo = Geometry::of(&image);
    let mut lost = image.clone();
    geo.set_bit(&mut lost, geo.count - 5, true);
    geo.set_bit(&mut lost, geo.count - 4, true);
    assert_eq!(kinds(&lost), [FindingKind::LostClusters]);
    let (report, findings) = run(&lost, 4096);
    assert!(
        matches!(findings[..], [Finding::LostClusters { first, count: 2, .. }] if first == geo.count - 5),
        "{findings:?}"
    );
    assert_eq!(report.lost_clusters(), 2);

    let mut bad = image.clone();
    geo.set_bit(&mut bad, geo.count - 5, true);
    geo.set_fat(&mut bad, geo.count - 5, 0xFFFF_FFF7);
    let (report, findings) = run(&bad, 4096);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(report.bad_clusters(), 1);

    let mut freed = image.clone();
    let set = geo.set(&freed, geo.root, "README.TXT");
    let first = le32(&freed, set[1] + 20);
    geo.set_bit(&mut freed, first, false);
    assert_eq!(kinds(&freed), [FindingKind::FreeInUse]);
}

#[test]
fn upcase_checksum_is_checked() {
    let mut image = base();
    let geo = Geometry::of(&image);
    let at = geo.root_entries(&image, 0x82)[0];
    put32(&mut image, at + 4, 0x1234_5678);
    assert_eq!(kinds(&image), [FindingKind::UpcaseTable]);
}

#[test]
fn boot_region_damage_is_found() {
    let image = base();
    let mut checksum = image.clone();
    checksum[11 * 512] ^= 1;
    assert!(kinds(&checksum).contains(&FindingKind::BootChecksum));
    let mut backup = image.clone();
    backup[12 * 512 + 100] ^= 1;
    assert_eq!(kinds(&backup), [FindingKind::BackupBootRegion]);
    let mut extended = image.clone();
    extended[2 * 512 - 1] = 0;
    assert!(kinds(&extended).contains(&FindingKind::BootSector));
    let mut dirty = image.clone();
    dirty[106] |= 2;
    assert_eq!(kinds(&dirty), [FindingKind::VolumeDirty]);
    let mut percent = image.clone();
    percent[112] = 77;
    assert_eq!(kinds(&percent), [FindingKind::PercentInUse]);
    let mut unknown = image.clone();
    unknown[112] = 0xFF;
    assert!(kinds(&unknown).is_empty());
    let mut fat = image.clone();
    let geo = Geometry::of(&fat);
    geo.set_fat(&mut fat, 0, 0);
    assert_eq!(kinds(&fat), [FindingKind::FatEntries]);
}

#[test]
fn entry_set_damage_is_found() {
    let image = base();
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "lower.txt");

    let mut sum = image.clone();
    sum[set[0] + 2] ^= 1;
    assert_eq!(kinds(&sum), [FindingKind::SetChecksum]);

    let mut hash = image.clone();
    hash[set[1] + 4] ^= 1;
    geo.reseal(&mut hash, &set);
    assert_eq!(kinds(&hash), [FindingKind::NameHash]);

    let mut bad_name = image.clone();
    bad_name[set[2] + 2] = b'*';
    geo.reseal(&mut bad_name, &set);
    let found = kinds(&bad_name);
    assert!(found.contains(&FindingKind::BadName), "{found:?}");

    let mut vdl = image.clone();
    vdl[set[1] + 8..set[1] + 16].copy_from_slice(&1000u64.to_le_bytes());
    geo.reseal(&mut vdl, &set);
    assert_eq!(kinds(&vdl), [FindingKind::ValidDataLength]);

    let mut broken = image.clone();
    broken[set[0] + 1] = 5;
    let found = kinds(&broken);
    assert!(found.contains(&FindingKind::EntrySet), "{found:?}");

    let dir = geo.set(&image, geo.root, "Nested Dir");
    let mut size = image.clone();
    size[dir[1] + 24] = 1;
    size[dir[1] + 8] = 1;
    geo.reseal(&mut size, &dir);
    assert!(kinds(&size).contains(&FindingKind::DirectorySize));

    let mut label = image.clone();
    let at = geo.root_entries(&label, 0x83)[0];
    label[at + 1] = 12;
    assert_eq!(kinds(&label), [FindingKind::RootEntry]);
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
    assert!(found.contains(&FindingKind::CyclicChain), "{found:?}");

    let mut broken = image.clone();
    geo.set_fat(&mut broken, chain[2], 0);
    let found = kinds(&broken);
    assert!(
        found.contains(&FindingKind::BrokenChain) && found.contains(&FindingKind::LostClusters),
        "{found:?}"
    );

    let mut short = image.clone();
    geo.set_fat(&mut short, chain[2], 0xFFFF_FFFF);
    let found = kinds(&short);
    assert!(found.contains(&FindingKind::ChainTooShort), "{found:?}");

    let mut bad = image.clone();
    geo.set_fat(&mut bad, chain[2], 0xFFFF_FFF7);
    let found = kinds(&bad);
    assert!(found.contains(&FindingKind::BadCluster), "{found:?}");

    let readme = geo.set(&image, geo.root, "README.TXT");
    let mut cross = image.clone();
    put32(&mut cross, readme[1] + 20, chain[0]);
    geo.reseal(&mut cross, &readme);
    let found = kinds(&cross);
    assert!(found.contains(&FindingKind::CrossLinked), "{found:?}");

    let mut invalid = image.clone();
    put32(&mut invalid, readme[1] + 20, geo.count + 2);
    geo.reseal(&mut invalid, &readme);
    let found = kinds(&invalid);
    assert!(found.contains(&FindingKind::InvalidCluster), "{found:?}");

    let mut long = image.clone();
    let lower = geo.set(&long, geo.root, "lower.txt");
    long[lower[1] + 24..lower[1] + 32].copy_from_slice(&0u64.to_le_bytes());
    long[lower[1] + 8..lower[1] + 16].copy_from_slice(&0u64.to_le_bytes());
    geo.reseal(&mut long, &lower);
    let found = kinds(&long);
    assert!(found.contains(&FindingKind::ChainTooLong), "{found:?}");
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
    assert!(found.contains(&FindingKind::TooDeep), "{found:?}");
}

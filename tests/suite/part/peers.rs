//! GPT and hybrid MBR edits checked against GPT fdisk: `sgdisk -e`,
//! `sgdisk --resize-table` and `sgdisk -h` produce the bytes Hadris does,
//! and `sgdisk -v` accepts every table Hadris writes.

use std::path::Path;

use hadris_part::gpt::types;
use hadris_part::sync::{read, write};
use hadris_part::{Disk, DiskLayout, Guid, Hybrid, PartitionSpec, PartitionTable, Size};
use hadris_storage::{BlockSize, MemDevice};
use hadris_tests::harness::{Workspace, require_or_skip};
use hadris_tests::part::{FORMAT, sgdisk, verify};

const B512: BlockSize = BlockSize::new(512).unwrap();
const MIB: u64 = 1 << 20;

fn sgdisk_available() -> bool {
    require_or_skip("sgdisk", "--version")
}

fn load(image: &Path) -> Disk {
    let mut dev = MemDevice::new(std::fs::read(image).unwrap(), B512);
    read(&mut dev).unwrap()
}

fn store(image: &Path, disk: &Disk) {
    let mut dev = MemDevice::new(std::fs::read(image).unwrap(), B512);
    write(&mut dev, disk).unwrap();
    std::fs::write(image, dev.get_ref()).unwrap();
}

fn grow(image: &Path, size: u64) {
    let file = std::fs::OpenOptions::new().write(true).open(image).unwrap();
    file.set_len(size).unwrap();
}

/// An 8 MiB image with a two-partition GPT written by Hadris.
fn hadris_image(image: &Path) {
    let disk = DiskLayout::gpt(Guid::from_bytes([0x5A; 16]))
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(1)).with_name("EFI"))
        .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::MiB(2)).with_name("root"))
        .build(8 * MIB / 512, B512)
        .unwrap();
    std::fs::write(image, vec![0u8; 8 * MIB as usize]).unwrap();
    store(image, &disk);
}

fn gpt_mut(disk: &mut Disk) -> &mut hadris_part::Gpt {
    match disk.table_mut() {
        PartitionTable::Gpt(gpt) => gpt,
        PartitionTable::Hybrid(hybrid) => hybrid.gpt_mut(),
        other => panic!("expected a GPT, found {:?}", other.kind()),
    }
}

/// Copies `source` to `ours` and `theirs`, edits `ours` with `edit` and
/// `theirs` with `sgdisk args`, and requires byte-identical images that
/// `sgdisk -v` accepts.
fn compare(dir: &Path, source: &Path, args: &[&str], edit: impl FnOnce(&mut Disk)) {
    let ours = dir.join("ours.img");
    let theirs = dir.join("theirs.img");
    std::fs::copy(source, &ours).unwrap();
    std::fs::copy(source, &theirs).unwrap();
    sgdisk(args, &theirs).unwrap();
    let mut disk = load(&ours);
    edit(&mut disk);
    store(&ours, &disk);
    verify(&ours).unwrap();
    let (a, b) = (
        std::fs::read(&ours).unwrap(),
        std::fs::read(&theirs).unwrap(),
    );
    let first = a.iter().zip(&b).position(|(x, y)| x != y);
    assert_eq!(
        first, None,
        "images differ from byte {first:?} after {args:?}"
    );
}

#[test]
fn relocate_backup_matches_sgdisk_e() {
    if !sgdisk_available() {
        return;
    }
    let workspace = Workspace::new(FORMAT, "relocate-").unwrap();
    let source = workspace.path.join("source.img");
    hadris_image(&source);
    grow(&source, 16 * MIB);
    compare(&workspace.path, &source, &["-e"], |disk| {
        let gpt = gpt_mut(disk);
        gpt.relocate_backup(16 * MIB / 512).unwrap();
        assert_eq!(gpt.last_usable(), 16 * MIB / 512 - 34);
    });
}

#[test]
fn set_entry_count_matches_sgdisk_resize_table() {
    if !sgdisk_available() {
        return;
    }
    let workspace = Workspace::new(FORMAT, "resize-table-").unwrap();
    let source = workspace.path.join("source.img");
    hadris_image(&source);
    for count in [8usize, 256, 4096] {
        let arg = format!("--resize-table={count}");
        compare(&workspace.path, &source, &[&arg], |disk| {
            gpt_mut(disk).set_entry_count(count).unwrap();
        });
    }

    let mut disk = load(&source);
    let before = disk.clone();
    assert!(gpt_mut(&mut disk).set_entry_count(8192).is_err());
    assert_eq!(disk, before);
    let copy = workspace.path.join("overlap.img");
    std::fs::copy(&source, &copy).unwrap();
    let _ = sgdisk(&["--resize-table=8192"], &copy);
    assert_eq!(load(&copy), before, "sgdisk resized over a partition");
}

#[test]
fn sgdisk_hybrids_round_trip() {
    if !sgdisk_available() {
        return;
    }
    let workspace = Workspace::new(FORMAT, "hybrid-").unwrap();
    for spec in ["1:2:EE", "2:EE", "1:2"] {
        let image = workspace.path.join("hybrid.img");
        hadris_image(&image);
        sgdisk(&["-t", "1:ef00", "-t", "2:0700", "-h", spec], &image).unwrap();
        let disk = load(&image);
        let PartitionTable::Hybrid(hybrid) = disk.table() else {
            panic!("sgdisk -h {spec} did not read as a hybrid");
        };
        let rebuilt = Hybrid::new(hybrid.gpt().clone(), &hybrid.mirror()).unwrap();
        assert_eq!(&rebuilt, hybrid, "sgdisk -h {spec} does not round-trip");

        grow(&image, 16 * MIB);
        let mut disk = load(&image);
        let PartitionTable::Hybrid(hybrid) = disk.table_mut() else {
            unreachable!();
        };
        hybrid.gpt_mut().relocate_backup(16 * MIB / 512).unwrap();
        *hybrid = Hybrid::new(hybrid.gpt().clone(), &hybrid.mirror()).unwrap();
        store(&image, &disk);
        verify(&image).unwrap();
        assert_eq!(load(&image), disk);
    }
}

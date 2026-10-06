use std::process::{Command, Output};

#[allow(dead_code)]
#[path = "../../../block/hadris-apfs/tests/common/mod.rs"]
mod common;

fn run(image: &std::path::Path, command: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["apfs", command])
        .arg(image)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn container_inspection_and_selected_volume_reads() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("container.apfs");
    std::fs::write(&image, common::multi_volume_image()).unwrap();
    let info = run(&image, "info", &[]);
    assert!(info.status.success(), "{:?}", info);
    assert!(String::from_utf8_lossy(&info.stdout).contains("Other"));
    assert!(!run(&image, "ls", &["/"]).status.success());
    let list = run(&image, "ls", &["/", "--volume-name", "Other"]);
    assert!(list.status.success(), "{:?}", list);
    assert!(String::from_utf8_lossy(&list.stdout).contains("file0.txt"));
    let cat = run(&image, "cat", &["/file0.txt", "--volume-object-id", "2048"]);
    assert!(cat.status.success(), "{:?}", cat);
    assert_eq!(cat.stdout, common::file_contents(0));
    assert!(
        run(&image, "stat", &["/file0.txt", "--volume-name", "Other"])
            .status
            .success()
    );
    let out = dir.path().join("extracted");
    let extracted = run(
        &image,
        "extract",
        &["/", "--volume-name", "Other", "-o", out.to_str().unwrap()],
    );
    assert!(extracted.status.success(), "{:?}", extracted);
    assert_eq!(
        std::fs::read(out.join("file0.txt")).unwrap(),
        common::file_contents(0)
    );
}

#[test]
fn symlink_cat_and_hardlink_extraction_use_shared_driver() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("container.apfs");
    std::fs::write(&image, common::use_case_image()).unwrap();
    let cat = run(&image, "cat", &["/link"]);
    assert!(cat.status.success(), "{:?}", cat);
    assert_eq!(cat.stdout, common::file_contents(0));
    let out = dir.path().join("extracted");
    let extracted = run(
        &image,
        "extract",
        &["/alias.txt", "-o", out.to_str().unwrap()],
    );
    assert!(extracted.status.success(), "{:?}", extracted);
    assert_eq!(
        std::fs::read(out.join("alias.txt")).unwrap(),
        common::file_contents(0)
    );
    let repeated = run(
        &image,
        "extract",
        &["/alias.txt", "-o", out.to_str().unwrap()],
    );
    assert!(!repeated.status.success());
    assert_eq!(
        std::fs::read(out.join("alias.txt")).unwrap(),
        common::file_contents(0)
    );
}

#[test]
fn whole_disk_partition_selection_is_explicit_when_ambiguous() {
    use hadris::part::{Disk, Gpt, GptEntry, Guid};
    use hadris::storage::{BlockSize, MemDevice};
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("disk.img");
    let block_size = BlockSize::new(512).unwrap();
    let mut device = MemDevice::new(vec![0u8; 4096 * 512], block_size);
    let mut gpt = Gpt::new(Guid::from_bytes([1; 16]), 4096, block_size).unwrap();
    let container = common::build_image();
    let length = (container.len() / 512) as u64;
    for (ordinal, start) in [2048u64, 3072].into_iter().enumerate() {
        gpt.add(GptEntry::new(
            hadris::part::gpt::types::APPLE_APFS,
            Guid::from_bytes([2 + ordinal as u8; 16]),
            start,
            length,
        ))
        .unwrap();
        let begin = start as usize * 512;
        device.get_mut()[begin..begin + container.len()].copy_from_slice(&container);
    }
    hadris::part::sync::write(&mut device, &Disk::new(gpt)).unwrap();
    std::fs::write(&image, device.into_inner()).unwrap();
    assert!(!run(&image, "info", &["--gpt"]).status.success());
    let cat = run(&image, "cat", &["/file0.txt", "--partition", "1"]);
    assert!(cat.status.success(), "{:?}", cat);
    assert_eq!(cat.stdout, common::file_contents(0));
}

//! Qualification against native exFAT implementations: volumes made by
//! exfatprogs `mkfs.exfat` and macOS `newfs_exfat` mount and take writes
//! that `fsck.exfat` and `fsck_exfat` accept, and the macOS kernel driver
//! reads what `ExFatFs` writes and the other way round. Each test skips a
//! tool that is not installed.

#[path = "common/exfat.rs"]
mod common;
use common::FsPaths;

use common::{clean, fsck};
use hadris_fs::sync::FileSystem;
use hadris_fs::{Name, RenameMode};

fn exercise(image: &[u8], label: &str, what: &str) -> Vec<u8> {
    let mut fs = common::mount(image);
    assert_eq!(fs.label_text().unwrap().unwrap(), label, "{what}");
    clean(&mut fs, what);
    let root = fs.root();
    let dir = common::mkdir(&mut fs, root, "Hadris Dir");
    for i in 0..150 {
        let node = common::write(
            &mut fs,
            dir,
            &format!("written by hadris {i:03}.txt"),
            &common::payload(i * 97, i as u8),
        );
        fs.forget(node, 1);
    }
    let big = common::write(&mut fs, root, "big.bin", &common::payload(3 << 20, 1));
    fs.truncate(big, 1 << 20).unwrap();
    fs.forget(big, 1);
    fs.rename(
        dir,
        Name::new("written by hadris 000.txt"),
        root,
        Name::new("moved.txt"),
        RenameMode::Replace,
    )
    .unwrap();
    fs.unlink(dir, Name::new("written by hadris 001.txt"))
        .unwrap();
    fs.forget(dir, 1);
    fs.set_label(Some(
        hadris_fat::exfat::VolumeLabel::new("Relabeled").unwrap(),
    ))
    .unwrap();
    fs.sync().unwrap();
    clean(&mut fs, what);
    let image = common::image(fs);
    fsck(&image, what);
    image
}

#[test]
fn mkfs_exfat_images_are_read_and_written() {
    let Some(image) = common::mkfs_exfat(64 << 20, "EXFATPROGS") else {
        eprintln!("mkfs.exfat is not available");
        return;
    };
    exercise(&image, "EXFATPROGS", "mkfs.exfat image");
}

#[test]
fn newfs_exfat_images_are_read_and_written() {
    let Some(image) = common::newfs_exfat(32 << 20, "NEWFS") else {
        eprintln!("newfs_exfat is not available");
        return;
    };
    exercise(&image, "NEWFS", "newfs_exfat image");
}

#[test]
fn the_macos_kernel_reads_what_hadris_writes() {
    let image = exercise(&common::build(), "Hadris", "built image");
    let mut fs = common::mount(&image);
    let expected: Vec<(String, Vec<u8>)> = common::names(&mut fs, "/Hadris Dir")
        .into_iter()
        .map(|n| {
            let data = fs.read_to_vec(&format!("/Hadris Dir/{n}")).unwrap();
            (n, data)
        })
        .collect();
    let big = fs.read_to_vec("/big.bin").unwrap();
    let deep = fs.read_to_vec("/Nested Dir/inner/deep.bin").unwrap();
    drop(fs);
    let kernel = common::with_macos_mount(&image, |mount| {
        for (n, data) in &expected {
            assert_eq!(
                &std::fs::read(mount.join("Hadris Dir").join(n)).unwrap(),
                data,
                "{n}"
            );
        }
        assert_eq!(std::fs::read(mount.join("big.bin")).unwrap(), big);
        assert_eq!(
            std::fs::read(mount.join("Nested Dir/inner/deep.bin")).unwrap(),
            deep
        );
        assert_eq!(
            std::fs::read_dir(mount.join("Nested Dir/inner"))
                .unwrap()
                .count(),
            301
        );
        std::fs::create_dir(mount.join("from kernel")).unwrap();
        for i in 0..40 {
            std::fs::write(
                mount.join(format!("from kernel/kernel file {i}.dat")),
                common::payload(i * 1000, 9),
            )
            .unwrap();
        }
        std::fs::remove_file(mount.join("moved.txt")).unwrap();
        std::fs::rename(
            mount.join("big.bin"),
            mount.join("from kernel/renamed big.bin"),
        )
        .unwrap();
    });
    let Some(kernel) = kernel else {
        eprintln!("hdiutil is not available");
        return;
    };
    let mut fs = common::mount(&kernel);
    for i in 0..40 {
        assert_eq!(
            fs.read_to_vec(&format!("/from kernel/kernel file {i}.dat"))
                .unwrap(),
            common::payload(i * 1000, 9)
        );
    }
    assert_eq!(fs.read_to_vec("/from kernel/renamed big.bin").unwrap(), big);
    assert!(fs.resolve_path("/moved.txt").is_err());
    let (_, findings) = common::check_dev(&mut common::device(kernel.clone(), 512), 4096);
    assert!(
        findings
            .iter()
            .all(|f| f.detail == hadris_fat::exfat::Detail::PercentInUse),
        "{findings:?}"
    );
    let root = fs.root();
    let node = common::write(&mut fs, root, "after kernel.txt", b"again");
    fs.forget(node, 1);
    fs.sync().unwrap();
    fsck(&common::image(fs), "after the macOS kernel");
}

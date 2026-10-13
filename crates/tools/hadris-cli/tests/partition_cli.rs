//! `hadris ls|cat|extract|stat`, and `--partition`, `--offset` and
//! `--backup-boot` on the format commands.

use std::path::Path;
use std::process::{Command, Output};

use hadris::fs::sync::FileSystem;
use hadris::fs::{Content, MountOptions, Name, Node, OpenMode, SetAttr, Tree};
use hadris::part::{self, DiskLayout, Guid, MbrType, PartitionSpec, Size, gpt::types};
use hadris::storage::{BlockSize, MemDevice};
use hadris::{Select, sync::open_partition};

type Dev = MemDevice<Vec<u8>>;

fn hadris() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hadris"))
}

/// Runs the command `args` starts with on `image`, then the rest of `args`.
fn run(args: &[&str], image: &Path) -> Output {
    let words = if matches!(args[0], "fat" | "iso" | "udf") {
        2
    } else {
        1
    };
    let (command, rest) = args.split_at(words);
    hadris()
        .args(command)
        .arg(image)
        .args(rest)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn device(len: usize, block: u32) -> Dev {
    MemDevice::new(vec![0u8; len], BlockSize::new(block).unwrap())
}

fn put(dev: Dev, slot: usize, name: &str, data: &[u8]) -> Dev {
    let mut fs = open_partition(dev, Select::Index(slot), MountOptions::new()).unwrap();
    let root = fs.root();
    let node = fs.create(root, Name::new(name), &SetAttr::new()).unwrap();
    fs.open(node, OpenMode::Write).unwrap();
    fs.write(node, 0, data).unwrap();
    fs.close(node).unwrap();
    fs.forget(node, 1);
    fs.unmount().unwrap().into_inner()
}

/// A GPT disk: slot 0 a FAT volume holding `HELLO.TXT`, slot 1 an exFAT
/// volume holding `DATA.TXT`.
fn gpt_disk(path: &Path) -> u64 {
    let mut dev = device(16 << 20, 512);
    let layout = DiskLayout::gpt(Guid::from_bytes([0x33; 16]))
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(4)))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining));
    let disk = part::sync::create(&mut dev, &layout).unwrap();
    let (esp, data) = (disk.partition(0).unwrap(), disk.partition(1).unwrap());
    let fat =
        hadris::fat::FatOptions::new().with_label(hadris::fat::VolumeLabel::new("BOOT").unwrap());
    hadris::fat::sync::format(&mut part::sync::open(&mut dev, &esp).unwrap(), &fat).unwrap();
    let exfat = hadris::fat::exfat::ExFatOptions::new().with_serial(0xCAFE_F00D);
    hadris::fat::exfat::sync::format(&mut part::sync::open(&mut dev, &data).unwrap(), &exfat)
        .unwrap();
    let dev = put(dev, 0, "HELLO.TXT", b"hello from the esp");
    let dev = put(dev, 1, "DATA.TXT", b"data partition");
    std::fs::write(path, dev.into_inner()).unwrap();
    data.start() * 512
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("README.TXT", Node::file(Content::bytes(b"optical")))
        .unwrap();
    tree
}

/// An MBR disk whose slot 0 holds an ISO 9660 image and slot 1 a UDF one.
fn optical_disk(path: &Path) {
    let mut dev = device(16 << 20, 512);
    let layout = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::ISO9660, Size::MiB(4)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::MiB(4)));
    let disk = part::sync::create(&mut dev, &layout).unwrap();
    let mut bytes = dev.into_inner();
    for slot in [0, 1] {
        let mut image = device(4 << 20, 2048);
        if slot == 0 {
            hadris::iso::sync::write(&mut image, &tree(), &hadris::iso::IsoOptions::new()).unwrap();
        } else {
            hadris::udf::sync::write(&mut image, &tree(), &hadris::udf::UdfOptions::new()).unwrap();
        }
        let start = (disk.partition(slot).unwrap().start() * 512) as usize;
        let image = image.into_inner();
        bytes[start..start + image.len()].copy_from_slice(&image);
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn generic_commands_read_any_partition() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("disk.img");
    gpt_disk(&image);

    assert!(stdout(&run(&["ls"], &image)).contains("HELLO.TXT"));
    let listed = stdout(&run(&["ls", "/", "--partition", "1", "-l"], &image));
    assert!(
        listed.contains("DATA.TXT") && listed.contains("14"),
        "{listed}"
    );
    assert_eq!(
        stdout(&run(&["cat", "/HELLO.TXT", "--partition", "0"], &image)),
        "hello from the esp"
    );

    let stat = stdout(&run(&["stat", "/DATA.TXT", "--partition", "1"], &image));
    assert!(stat.contains("Format:      exFAT"), "{stat}");
    assert!(stat.contains("Serial:      CAFEF00D"), "{stat}");
    assert!(stat.contains("Type:        file"), "{stat}");
    assert!(stat.contains("Size:        14 bytes"), "{stat}");
    let stat = stdout(&run(&["stat"], &image));
    assert!(stat.contains("Label:       BOOT"), "{stat}");

    let out = temp.path().join("out");
    let output = hadris()
        .args(["extract", "--partition", "1", "-o"])
        .arg(&out)
        .arg(&image)
        .output()
        .unwrap();
    stdout(&output);
    assert_eq!(
        std::fs::read(out.join("DATA.TXT")).unwrap(),
        b"data partition"
    );

    let missing = run(&["ls", "/", "--partition", "5"], &image);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no such partition"));
}

#[test]
fn fat_commands_take_a_partition_or_offset() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("disk.img");
    let offset = gpt_disk(&image);

    assert!(stdout(&run(&["fat", "ls", "--partition", "0"], &image)).contains("HELLO.TXT"));
    let info = stdout(&run(&["fat", "info", "--partition", "1"], &image));
    assert!(info.contains("Volume ID:       CAFEF00D"), "{info}");
    let offset = offset.to_string();
    assert!(stdout(&run(&["fat", "ls", "--offset", &offset], &image)).contains("DATA.TXT"));
    assert_eq!(
        stdout(&run(
            &["fat", "cat", "/DATA.TXT", "--partition", "1"],
            &image
        )),
        "data partition"
    );
    let verify = run(&["fat", "verify", "--partition", "1"], &image);
    assert!(stdout(&verify).contains("PASS"));

    let whole = run(&["fat", "ls"], &image);
    assert!(!whole.status.success());
    assert!(String::from_utf8_lossy(&whole.stderr).contains("No FAT or exFAT volume"));
    let unaligned = run(&["fat", "ls", "--offset", "100"], &image);
    assert!(!unaligned.status.success());
    let both = run(&["fat", "ls", "--offset", "0", "--partition", "0"], &image);
    assert!(!both.status.success());
}

#[test]
fn backup_boot_mounts_a_volume_whose_primary_boot_sector_is_gone() {
    let temp = tempfile::tempdir().unwrap();
    for (kind, len) in [("exfat", "8388608"), ("fat32", "67108864")] {
        let source = temp.path().join(format!("{kind}-source"));
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a.txt"), b"backup").unwrap();
        let image = temp.path().join(format!("{kind}.img"));
        let created = hadris()
            .args(["fat", "create", "--fat-type", kind, "--size", len, "-o"])
            .arg(&image)
            .arg(&source)
            .output()
            .unwrap();
        stdout(&created);
        let mut bytes = std::fs::read(&image).unwrap();
        bytes[..512].fill(0);
        std::fs::write(&image, bytes).unwrap();

        assert!(!run(&["fat", "ls"], &image).status.success());
        assert!(stdout(&run(&["fat", "ls", "--backup-boot"], &image)).contains("a.txt"));
        assert_eq!(
            stdout(&run(&["fat", "cat", "/a.txt", "--backup-boot"], &image)),
            "backup"
        );
        stdout(&run(&["fat", "info", "--backup-boot"], &image));
    }
}

#[test]
fn optical_commands_take_a_partition() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("disk.img");
    optical_disk(&image);

    assert!(stdout(&run(&["iso", "ls", "--partition", "0"], &image)).contains("README.TXT"));
    assert_eq!(
        stdout(&run(
            &["iso", "cat", "/README.TXT", "--partition", "0"],
            &image
        )),
        "optical"
    );
    assert!(stdout(&run(&["udf", "ls", "--partition", "1"], &image)).contains("README.TXT"));
    assert_eq!(
        stdout(&run(
            &["udf", "cat", "/README.TXT", "--partition", "1"],
            &image
        )),
        "optical"
    );
    assert_eq!(
        stdout(&run(&["cat", "/README.TXT", "--partition", "1"], &image)),
        "optical"
    );
    assert!(stdout(&run(&["ls"], &image)).contains("README.TXT"));
}

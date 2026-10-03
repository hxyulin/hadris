//! Builds a Linux initramfs the way a distribution does: an uncompressed
//! early archive with CPU microcode, followed by the root filesystem, both
//! streamed to sinks that cannot seek. Then streams the result back through
//! a reader that cannot seek either, segment by segment, and rebuilds the
//! root archive from `read_tree` byte for byte.
//!
//! Catalog actions: BUILD-CPIO-INITRAMFS-01, BUILD-CPIO-FMT-01,
//! IO-STREAM-01, IO-STREAM-02, SESSION-CPIO-02, LINK-SYMLINK-01,
//! LINK-HARD-01, LINK-SPECIAL-01, META-PERM-01, BUILD-TREE-03,
//! NF-DET-01.
//!
//! ```text
//! cargo run -p hadris-example-initramfs                     # build and check
//! cargo run -p hadris-example-initramfs -- initramfs.cpio   # also write it
//! ```

use anyhow::{Context, Result, ensure};
use hadris::cpio::CpioOptions;
use hadris::cpio::sync::{CpioReader, read_tree};
use hadris::fs::{
    Content, DateTime, DeviceNumber, FileType, Node, Owner, Permissions, SetAttr, Tree,
};
use hadris::host::StdIo;
use hadris::io::sync::Read;

const MICROCODE: &[u8] = b"stand-in for Intel microcode";
const BUSYBOX: &[u8] = b"stand-in for a static busybox";
const INIT: &[u8] = b"#!/bin/sh\nmount -t devtmpfs dev /dev\nexec /bin/sh\n";

fn main() -> Result<()> {
    let options = CpioOptions::new()
        .with_time(DateTime::from_unix_seconds(1_700_000_000).context("bad time")?);
    let early = archive(&early_tree()?, &options)?;
    let root = archive(&root_tree()?, &options)?;
    ensure!(
        root == archive(&root_tree()?, &options)?,
        "the same tree and options should give the same bytes"
    );

    let mut initrd = early.clone();
    initrd.extend_from_slice(&root);
    check(&initrd)?;

    let rebuilt = read_tree(&mut CpioReader::new(StdIo::new(&root[..])))?;
    ensure!(
        archive(&rebuilt, &options)? == root,
        "read_tree should keep every entry"
    );

    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(&path, &initrd)?;
        println!("wrote {}", path.to_string_lossy());
    }
    println!("initramfs built and checked");
    Ok(())
}

fn early_tree() -> Result<Tree> {
    let mut tree = Tree::new();
    tree.insert(
        "kernel/x86/microcode/GenuineIntel.bin",
        Node::file(Content::bytes(MICROCODE)),
    )?;
    Ok(tree)
}

fn root_tree() -> Result<Tree> {
    let root = |mode| {
        SetAttr::new()
            .with_owner(Owner::new(0, 0))
            .with_permissions(Permissions::new(mode))
    };
    let mut tree = Tree::new();
    tree.insert(
        "init",
        Node::file(Content::bytes(INIT)).with_attrs(root(0o755)),
    )?;
    tree.insert(
        "bin/busybox",
        Node::file(Content::bytes(BUSYBOX)).with_attrs(root(0o4755)),
    )?;
    tree.link("bin/busybox", "bin/mount")?;
    tree.insert("bin/sh", Node::symlink("busybox"))?;
    for (name, major, minor) in [("console", 5, 1), ("null", 1, 3)] {
        tree.insert(
            format!("dev/{name}"),
            Node::special(FileType::CharDevice, Some(DeviceNumber::new(major, minor)))
                .with_attrs(root(0o600)),
        )?;
    }
    tree.insert("etc/hostname", Node::file(Content::bytes("hadris\n")))?;
    Ok(tree)
}

/// Writes `tree` to a `Vec` through `std::io::Write` only, as to a pipe.
fn archive(tree: &Tree, options: &CpioOptions) -> Result<Vec<u8>> {
    let mut sink = StdIo::new(Vec::new());
    hadris::cpio::sync::write(&mut sink, tree, options)?;
    Ok(sink.into_inner())
}

struct Seen {
    path: String,
    file_type: FileType,
    mode: u32,
    ino: u64,
    rdev: (u32, u32),
    data: Vec<u8>,
}

fn check(initrd: &[u8]) -> Result<()> {
    let mut reader = CpioReader::new(StdIo::new(initrd));
    let mut segments = Vec::new();
    loop {
        let mut seen = Vec::new();
        while let Some(mut entry) = reader.next_entry()? {
            let mut item = Seen {
                path: entry.path_str()?.to_owned(),
                file_type: entry.file_type(),
                mode: entry.mode(),
                ino: entry.ino(),
                rdev: (entry.rdev().major(), entry.rdev().minor()),
                data: vec![0; usize::try_from(entry.len())?],
            };
            entry.read_exact(&mut item.data)?;
            seen.push(item);
        }
        segments.push(seen);
        if !reader.next_segment()? {
            break;
        }
    }

    ensure!(segments.len() == 2, "expected the early and root segments");
    let early = &segments[0];
    let microcode = early
        .iter()
        .find(|e| e.path == "kernel/x86/microcode/GenuineIntel.bin")
        .context("the early segment has no microcode")?;
    ensure!(microcode.data == MICROCODE);

    let root = &segments[1];
    let get = |path: &str| {
        root.iter()
            .find(|e| e.path == path)
            .with_context(|| format!("{path} is missing"))
    };
    ensure!(get("init")?.mode == 0o100755 && get("init")?.data == INIT);
    ensure!(
        get("bin/busybox")?.mode == 0o104755,
        "setuid should survive"
    );
    ensure!(
        get("bin/busybox")?.ino == get("bin/mount")?.ino,
        "hard link"
    );
    let with_data: Vec<_> = ["bin/busybox", "bin/mount"]
        .into_iter()
        .filter(|path| get(path).is_ok_and(|e| !e.data.is_empty()))
        .collect();
    ensure!(
        with_data.len() == 1,
        "a hard link group stores its data once"
    );
    let sh = get("bin/sh")?;
    ensure!(sh.file_type == FileType::Symlink && sh.data == b"busybox");
    let console = get("dev/console")?;
    ensure!(console.file_type == FileType::CharDevice && console.rdev == (5, 1));
    ensure!(get("dev/null")?.rdev == (1, 3));
    Ok(())
}

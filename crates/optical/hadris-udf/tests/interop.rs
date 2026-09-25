//! Volumes read and written against external tools: `mkudffs` and
//! `udfinfo` (udftools) and 7-Zip. A test whose tool is missing prints a
//! note and passes, unless `HADRIS_REQUIRE_EXTERNAL_TOOLS` is set.

mod common;

use hadris_fs::MountOptions;
use std::path::Path;
use std::process::Command;

use common::Paths;
use common::{image, pattern};
use hadris_fs::{Content, Node, Tree};
use hadris_udf::sync::UdfFs;
use hadris_udf::{UdfOptions, UdfRevision};

fn tool(names: &[&'static str], probe: &str) -> Option<&'static str> {
    let found = names.iter().copied().find(|name| {
        Command::new(name)
            .arg(probe)
            .output()
            .is_ok_and(|out| out.status.code().is_some())
    });
    if found.is_none() {
        assert!(
            std::env::var_os("HADRIS_REQUIRE_EXTERNAL_TOOLS").is_none(),
            "{names:?} is required but missing"
        );
        eprintln!("skipping: {names:?} not installed");
    }
    found
}

/// Files and directories 7-Zip lists; it refuses volumes with symlinks.
fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("readme.txt", Node::file(Content::bytes("hello")))
        .unwrap();
    tree.insert(
        "docs/big.bin",
        Node::file(Content::bytes(pattern(70_000, 3))),
    )
    .unwrap();
    tree.insert("docs/caf\u{e9}.txt", Node::file(Content::bytes("latin1")))
        .unwrap();
    tree.insert("empty", Node::dir()).unwrap();
    tree
}

fn write(dir: &Path, revision: UdfRevision) -> std::path::PathBuf {
    let options = UdfOptions::default()
        .with_volume_id("INTEROP")
        .with_revision(revision);
    let path = dir.join(format!("hadris-{revision}.udf"));
    std::fs::write(&path, image(&tree(), &options)).unwrap();
    path
}

#[test]
fn udfinfo_accepts_hadris_volumes() {
    let Some(udfinfo) = tool(&["udfinfo"], "--help") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for revision in [UdfRevision::V1_02, UdfRevision::V2_01] {
        let path = write(dir.path(), revision);
        let out = Command::new(udfinfo).arg(&path).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{revision}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.contains("label=INTEROP"), "{text}");
        assert!(text.contains(&format!("udfrev={revision}")), "{text}");
    }
}

#[test]
fn seven_zip_extracts_hadris_volumes() {
    let Some(seven) = tool(&["7z", "7zz"], "i") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for revision in [UdfRevision::V1_02, UdfRevision::V2_01] {
        let path = write(dir.path(), revision);
        let out_dir = dir.path().join(format!("out-{revision}"));
        let out = Command::new(seven)
            .args(["x", "-tudf", "-y"])
            .arg(format!("-o{}", out_dir.display()))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(std::fs::read(out_dir.join("readme.txt")).unwrap(), b"hello");
        assert_eq!(
            std::fs::read(out_dir.join("docs/big.bin")).unwrap(),
            pattern(70_000, 3)
        );
        assert_eq!(
            std::fs::read(out_dir.join("docs/caf\u{e9}.txt")).unwrap(),
            b"latin1"
        );
        assert!(out_dir.join("empty").is_dir());
    }
}

#[test]
fn mkudffs_volumes_read_back() {
    let Some(mkudffs) = tool(&["mkudffs"], "--help") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for (revision, block) in [
        ("1.02", 2048),
        ("1.50", 2048),
        ("2.01", 2048),
        ("2.01", 512),
    ] {
        let path = dir.path().join(format!("mk-{revision}-{block}.udf"));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(8 << 20).unwrap();
        drop(file);
        let out = Command::new(mkudffs)
            .args([
                "--media-type=hd",
                "--label=MKUDFFS",
                &format!("--udfrev={revision}"),
            ])
            .arg(format!("--blocksize={block}"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut udf = UdfFs::mount(
            hadris_storage::host::FileDevice::open(&path).unwrap(),
            MountOptions::new(),
        )
        .unwrap_or_else(|err| panic!("{revision}/{block}: {err}"));
        assert_eq!(udf.logical_volume_id(), "MKUDFFS");
        assert_eq!(udf.block_size(), block);
        assert_eq!(udf.revision().to_string(), revision);
        let listed = udf.names("/").unwrap();
        assert!(listed.len() <= 1, "{revision}/{block}: {listed:?}");
    }
}

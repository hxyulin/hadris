//! Volumes read and written against external tools: `mkudffs` and
//! `udfinfo` (udftools), 7-Zip, and pycdlib for bridge images. A test whose
//! tool is missing prints a note and passes, unless
//! `HADRIS_REQUIRE_EXTERNAL_TOOLS` is set.

mod common;

use hadris_fs::MountOptions;
use std::path::Path;
use std::process::Command;

use common::Paths;
use common::{SECTOR, image, pattern};
use hadris_fs::{Content, Node, Tree};
use hadris_iso::{IsoLevel, IsoOptions};
use hadris_storage::MemDevice;
use hadris_udf::sync::UdfFs;
use hadris_udf::{UdfId, UdfOptions, UdfRevision};

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
        .with_id(UdfId::Volume, "INTEROP")
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
        assert_eq!(udf.info().id(hadris_udf::UdfId::LogicalVolume), "MKUDFFS");
        assert_eq!(udf.info().block_size(), block);
        assert_eq!(udf.info().revision().to_string(), revision);
        let listed = udf.names("/").unwrap();
        assert!(listed.len() <= 1, "{revision}/{block}: {listed:?}");
    }
}

/// Opens each image with pycdlib, failing on any warning it logs, and
/// prints `/readme.txt` read through the UDF namespace.
const PYCDLIB: &str = r#"
import logging, sys
import pycdlib

class Fail(logging.Handler):
    def emit(self, record):
        sys.exit("pycdlib: " + record.getMessage())

logging.getLogger("pycdlib").addHandler(Fail(logging.WARNING))
iso = pycdlib.PyCdlib()
iso.open(sys.argv[1])
assert iso.has_udf()
out = __import__("io").BytesIO()
iso.get_file_from_iso_fp(out, udf_path="/readme.txt")
sys.stdout.write(out.getvalue().decode())
iso.close()
"#;

#[test]
fn pycdlib_reads_hadris_bridges() {
    let has_pycdlib = Command::new("python3")
        .args(["-I", "-c", "import pycdlib"])
        .output()
        .is_ok_and(|out| out.status.success());
    if !has_pycdlib {
        assert!(
            std::env::var_os("HADRIS_REQUIRE_EXTERNAL_TOOLS").is_none(),
            "pycdlib is required but missing"
        );
        eprintln!("skipping: pycdlib not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for revision in [UdfRevision::V1_02, UdfRevision::V2_01] {
        let iso = IsoOptions::default()
            .with_level(IsoLevel::L3)
            .with_joliet()
            .with_rock_ridge();
        let udf = UdfOptions::default().with_revision(revision);
        let report = hadris_udf::plan_bridge(&tree(), &iso, &udf).unwrap();
        let mut dev = MemDevice::new(vec![0u8; report.size() as usize], SECTOR);
        hadris_udf::sync::write_bridge(&mut dev, &tree(), &iso, &udf).unwrap();
        let path = dir.path().join(format!("bridge-{revision}.iso"));
        std::fs::write(&path, dev.into_inner()).unwrap();

        let out = Command::new("python3")
            .args(["-I", "-c", PYCDLIB])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{revision}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, b"hello", "{revision}");
    }
}

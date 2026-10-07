mod common;

use hadris_cpio::Format;
use hadris_cpio::sync::{CpioReader, read_tree};
use hadris_fs::{Content, Node, Tree};
use hadris_io::Cursor;
use std::fs;
use std::process::{Command, Stdio};

fn available() -> bool {
    match Command::new("cpio").arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("HADRIS_REQUIRE_EXTERNAL_TOOLS").is_none(),
                "cpio is required"
            );
            eprintln!("skipping native interoperability: cpio is unavailable");
            false
        }
        Err(err) => panic!("checking cpio: {err}"),
    }
}

#[test]
fn native_cpio_extracts_hadris_archives() {
    if !available() {
        return;
    }
    let mut tree = Tree::new();
    let data = (0..70_001).map(|i| (i * 37) as u8).collect::<Vec<_>>();
    tree.insert("dir/data", Node::file(Content::bytes(data.clone())))
        .unwrap();
    tree.link("dir/data", "alias").unwrap();
    tree.insert("empty", Node::file(Content::empty())).unwrap();
    #[cfg(unix)]
    tree.insert("link", Node::symlink("dir/data")).unwrap();
    for format in [Format::Newc, Format::Crc, Format::Odc] {
        let image = tempfile::NamedTempFile::new().unwrap();
        fs::write(image.path(), common::archive(&tree, format)).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let output = Command::new("cpio")
            .arg("-idmu")
            .current_dir(destination.path())
            .stdin(Stdio::from(fs::File::open(image.path()).unwrap()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{format:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for name in ["dir/data", "alias"] {
            let actual = fs::read(destination.path().join(name)).unwrap();
            assert!(
                actual == data,
                "{format:?}: {name} has {} bytes, expected {}",
                actual.len(),
                data.len()
            );
        }
        assert!(
            fs::read(destination.path().join("empty"))
                .unwrap()
                .is_empty()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(destination.path().join("dir/data"))
                    .unwrap()
                    .ino(),
                fs::metadata(destination.path().join("alias"))
                    .unwrap()
                    .ino()
            );
            assert_eq!(
                fs::read_link(destination.path().join("link")).unwrap(),
                std::path::Path::new("dir/data")
            );
        }
    }
}

#[test]
fn hadris_reads_native_cpio_archives() {
    if !available() {
        return;
    }
    let source = tempfile::tempdir().unwrap();
    let data = vec![37; 70_001];
    fs::write(source.path().join("data"), &data).unwrap();
    fs::hard_link(source.path().join("data"), source.path().join("alias")).unwrap();
    fs::write(source.path().join("empty"), []).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("data", source.path().join("link")).unwrap();
    let names = tempfile::NamedTempFile::new().unwrap();
    let list: &[u8] = if cfg!(unix) {
        b"data\nalias\nempty\nlink\n"
    } else {
        b"data\nalias\nempty\n"
    };
    fs::write(names.path(), list).unwrap();
    let version = Command::new("cpio").arg("--version").output().unwrap();
    let formats: &[&str] = if String::from_utf8_lossy(&version.stdout).contains("GNU cpio") {
        &["newc", "crc", "odc"]
    } else {
        &["newc", "odc"]
    };
    for &format in formats {
        let output = Command::new("cpio")
            .args(["-o", "-H", format])
            .current_dir(source.path())
            .stdin(Stdio::from(fs::File::open(names.path()).unwrap()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{format}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let tree = read_tree(&mut CpioReader::new(Cursor::new(&output.stdout))).unwrap();
        for name in ["data", "alias"] {
            assert_eq!(
                tree.get(name).unwrap().content().unwrap().as_bytes(),
                Some(data.as_slice())
            );
            assert_eq!(tree.entry(name).unwrap().links(), 2);
        }
        assert_eq!(
            tree.entry("data").unwrap().id(),
            tree.entry("alias").unwrap().id()
        );
        assert!(tree.get("empty").unwrap().content().unwrap().is_empty());
        #[cfg(unix)]
        assert_eq!(
            tree.get("link").unwrap().target(),
            Some(&b"data"[..]),
            "{format}"
        );
    }
}

#![cfg(all(target_os = "macos", feature = "std", feature = "sync"))]
//! Reads APFS images that macOS builds and fills with `hdiutil`: nested and
//! large directories, files with holes, compressed files, symlinks, hard
//! links, and case-insensitive and case-sensitive names.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use hadris_apfs::ApfsError;
use hadris_apfs::sync::Container;
use hadris_apfs::types::VolumeSuperblock;
use hadris_apfs::types::filesystem::DT_LNK;
use hadris_storage::sync::SeekBlockDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

const MANY: usize = 1500;
const MIB: u64 = 1 << 20;

fn hdiutil(args: &[&str]) {
    let status = Command::new("hdiutil").args(args).status().unwrap();
    assert!(status.success(), "hdiutil {args:?} failed");
}

/// A temporary APFS image, deleted on drop.
struct Image {
    dir: PathBuf,
    path: PathBuf,
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Detaches the image when population finishes or panics.
struct Mounted<'a>(&'a Path);

impl Drop for Mounted<'_> {
    fn drop(&mut self) {
        hdiutil(&["detach", "-quiet", self.0.to_str().unwrap()]);
    }
}

fn build(name: &str, fs_type: &str, populate: impl FnOnce(&Path, &Path)) -> Image {
    let dir = std::env::temp_dir().join(format!("hadris-apfs-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mount = dir.join("mnt");
    let scratch = dir.join("scratch");
    fs::create_dir_all(&mount).unwrap();
    fs::create_dir_all(&scratch).unwrap();
    let image = Image {
        path: dir.join("image.dmg"),
        dir,
    };
    let path = image.path.to_str().unwrap();
    hdiutil(&[
        "create", "-quiet", "-fs", fs_type, "-layout", "NONE", "-volname", name, "-size", "32m",
        path,
    ]);
    hdiutil(&[
        "attach",
        "-quiet",
        "-nobrowse",
        "-mountpoint",
        mount.to_str().unwrap(),
        path,
    ]);
    let mounted = Mounted(&mount);
    populate(&mount, &scratch);
    drop(mounted);
    let check = Command::new("/sbin/fsck_apfs")
        .arg("-n")
        .arg(&image.path)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "fsck_apfs failed: {}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    image
}

fn open(image: &Image) -> (Container<SeekBlockDevice<File>>, VolumeSuperblock) {
    let file = File::open(&image.path).unwrap();
    let blocks = file.metadata().unwrap().len() / 512;
    let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount(blocks));
    let mut container = Container::open(SeekBlockDevice::new(file, geometry)).unwrap();
    let latest = container.latest_superblock().unwrap();
    let mut volumes = container.volume_superblocks(&latest).unwrap();
    assert_eq!(volumes.len(), 1);
    (container, volumes.remove(0))
}

fn read(
    container: &mut Container<SeekBlockDevice<File>>,
    volume: &VolumeSuperblock,
    path: &str,
) -> Vec<u8> {
    let entry = container
        .resolve_path(volume, path)
        .unwrap()
        .unwrap_or_else(|| panic!("{path} not found"));
    container
        .read_file(volume, entry.file_id, usize::MAX)
        .unwrap()
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn sparse_contents() -> Vec<u8> {
    let mut data = vec![0_u8; (MIB + 64 * 1024) as usize];
    data[..4096].fill(b'A');
    data[MIB as usize..MIB as usize + 4096].fill(b'B');
    data
}

#[test]
fn reads_a_case_insensitive_volume_written_by_macos() {
    let image = build("HADRIS", "APFS", |root, scratch| {
        fs::write(root.join("hello.txt"), b"hello apfs\n").unwrap();
        fs::create_dir_all(root.join("deep/a/b/c")).unwrap();
        fs::write(root.join("deep/a/b/c/d.txt"), b"deep\n").unwrap();
        fs::write(root.join("big.bin"), pattern(300_000)).unwrap();
        fs::create_dir(root.join("many")).unwrap();
        for i in 0..MANY {
            fs::write(root.join(format!("many/f{i}.txt")), format!("file {i}\n")).unwrap();
        }

        let mut sparse = File::create(root.join("sparse.bin")).unwrap();
        sparse.write_all(&[b'A'; 4096]).unwrap();
        sparse.seek(SeekFrom::Start(MIB)).unwrap();
        sparse.write_all(&[b'B'; 4096]).unwrap();
        sparse.set_len(MIB + 64 * 1024).unwrap();
        drop(sparse);

        std::os::unix::fs::symlink("hello.txt", root.join("link")).unwrap();
        fs::write(root.join("first"), b"linked\n").unwrap();
        fs::hard_link(root.join("first"), root.join("second")).unwrap();

        let plain = scratch.join("plain.txt");
        fs::write(&plain, b"compress me please ".repeat(10_000)).unwrap();
        let status = Command::new("ditto")
            .arg("--hfsCompression")
            .arg(&plain)
            .arg(root.join("compressed.txt"))
            .status()
            .unwrap();
        assert!(status.success());
    });
    let (mut container, volume) = open(&image);
    assert!(volume.is_case_insensitive());
    assert_eq!(volume.name().unwrap(), "HADRIS");

    assert_eq!(read(&mut container, &volume, "/hello.txt"), b"hello apfs\n");
    assert_eq!(read(&mut container, &volume, "/HELLO.TXT"), b"hello apfs\n");
    assert_eq!(
        read(&mut container, &volume, "/Deep/A/b/C/d.txt"),
        b"deep\n"
    );
    assert_eq!(read(&mut container, &volume, "/big.bin"), pattern(300_000));

    let many = container.resolve_path(&volume, "/many").unwrap().unwrap();
    assert_eq!(
        container
            .directory_owned_entries(&volume, many.file_id)
            .unwrap()
            .len(),
        MANY
    );
    for i in [0, MANY / 2, MANY - 1] {
        assert_eq!(
            read(&mut container, &volume, &format!("/many/f{i}.txt")),
            format!("file {i}\n").as_bytes()
        );
    }

    let sparse = container
        .resolve_path(&volume, "/sparse.bin")
        .unwrap()
        .unwrap();
    let inode = container
        .inode_record(&volume, sparse.file_id)
        .unwrap()
        .unwrap();
    let extents = container.file_extents(&volume, inode.private_id).unwrap();
    assert!(
        extents.iter().any(|extent| extent.physical_block == 0),
        "macOS should store the holes as sparse extents: {extents:?}"
    );
    assert_eq!(
        read(&mut container, &volume, "/sparse.bin"),
        sparse_contents()
    );
    assert_eq!(
        container.file_size(&volume, sparse.file_id).unwrap(),
        MIB + 64 * 1024
    );

    let link = container.resolve_path(&volume, "/link").unwrap().unwrap();
    assert_eq!(link.file_type(), DT_LNK);
    assert_eq!(
        container
            .symlink_target(&volume, link.file_id)
            .unwrap()
            .as_deref(),
        Some("hello.txt")
    );

    let first = container.resolve_path(&volume, "/first").unwrap().unwrap();
    let second = container.resolve_path(&volume, "/second").unwrap().unwrap();
    assert_eq!(first.file_id, second.file_id);
    assert_eq!(read(&mut container, &volume, "/second"), b"linked\n");

    let compressed = container
        .resolve_path(&volume, "/compressed.txt")
        .unwrap()
        .unwrap();
    let inode = container
        .inode_record(&volume, compressed.file_id)
        .unwrap()
        .unwrap();
    assert!(
        inode.is_compressed(),
        "ditto should have compressed the file"
    );
    assert_eq!(
        container
            .read_file(&volume, compressed.file_id, usize::MAX)
            .unwrap_err(),
        ApfsError::Unsupported("compressed file data")
    );
}

#[test]
fn case_sensitive_volume_needs_the_exact_name() {
    let image = build("CASED", "Case-sensitive APFS", |root, _| {
        fs::write(root.join("Name.txt"), b"upper\n").unwrap();
        fs::write(root.join("name.txt"), b"lower\n").unwrap();
    });
    let (mut container, volume) = open(&image);
    assert!(!volume.is_case_insensitive());
    assert_eq!(read(&mut container, &volume, "/Name.txt"), b"upper\n");
    assert_eq!(read(&mut container, &volume, "/name.txt"), b"lower\n");
    assert!(
        container
            .resolve_path(&volume, "/NAME.TXT")
            .unwrap()
            .is_none()
    );
}

#[test]
fn path_components_follow_native_directory_rules() {
    let image = build("PATHS", "APFS", |root, _| {
        fs::write(root.join("file"), b"contents").unwrap();
        fs::create_dir(root.join("dir")).unwrap();
        assert!(fs::metadata(root.join("file/")).is_err());
        assert!(fs::metadata(root.join("file/.")).is_err());
        assert_eq!(fs::read(root.join("dir/../file")).unwrap(), b"contents");
    });
    let (mut container, volume) = open(&image);
    for path in ["/file/", "/file/.", "/file/../file"] {
        assert!(
            container.resolve_path(&volume, path).unwrap().is_none(),
            "{path}"
        );
    }
    for path in ["/./file", "/dir/../file", "/../../file"] {
        assert_eq!(read(&mut container, &volume, path), b"contents");
    }
    assert!(container.resolve_path(&volume, "/dir/").unwrap().is_some());
    for path in ["", "/", "/.", "/dir/..", "/dir/../.", "/../../"] {
        let root = container.resolve_path(&volume, path).unwrap().unwrap();
        assert_eq!(
            root.file_id,
            hadris_apfs::types::filesystem::INODE_ROOT_DIRECTORY
        );
        assert_eq!(root.parent_id, root.file_id);
        assert_eq!(root.file_type(), hadris_apfs::types::filesystem::DT_DIR);
        assert_eq!(root.name, "/");
    }
}

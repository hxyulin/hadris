#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

use hadris_fat::exfat::sync::{ExFatFs, format};
use hadris_fat::exfat::{FormatOptions, MountOptions, VolumeLabel};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::{DirCursor, HeapTable, Name, NameBuf, NewNode, NodeId, SetMetadata};
use hadris_storage::{BlockSize, MemDevice};

pub type Device = MemDevice<Vec<u8>>;
pub type Fs = ExFatFs<Device, HeapTable>;

pub const SECTOR: usize = 512;

pub fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
        .collect()
}

pub fn device(image: Vec<u8>, block: u32) -> Device {
    MemDevice::new(image, BlockSize::new(block).unwrap())
}

/// Formats `size` bytes with `options` and mounts the result with a heap
/// table.
pub fn formatted(size: usize, options: FormatOptions) -> Fs {
    let fs = format(device(vec![0u8; size], 512), options).unwrap();
    mount(&fs.into_inner().into_inner())
}

/// A volume of `size` bytes with `cluster`-byte clusters.
pub fn small(size: usize, cluster: u32) -> Fs {
    formatted(size, FormatOptions::new().with_cluster_size(cluster))
}

pub fn mount(image: &[u8]) -> Fs {
    ExFatFs::open_with(
        device(image.to_vec(), 512),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap()
}

pub fn image(fs: Fs) -> Vec<u8> {
    fs.into_inner().into_inner()
}

pub fn write(fs: &mut Fs, dir: NodeId, text: &str, data: &[u8]) -> NodeId {
    let node = fs
        .create(
            dir,
            Name::new(text).unwrap(),
            NewNode::File,
            &SetMetadata::new(),
        )
        .unwrap();
    append(fs, node, 0, data);
    node
}

pub fn write_any<
    D: hadris_storage::sync::BlockDevice,
    T: hadris_fs::NodeTable,
    C: hadris_fs::Clock,
>(
    fs: &mut ExFatFs<D, T, C>,
    dir: NodeId,
    text: &str,
    data: &[u8],
) -> NodeId {
    let node = fs
        .create(
            dir,
            Name::new(text).unwrap(),
            NewNode::File,
            &SetMetadata::new(),
        )
        .unwrap();
    assert_eq!(fs.write_at(node, 0, data).unwrap(), data.len());
    node
}

pub fn append(fs: &mut Fs, node: NodeId, mut at: u64, mut data: &[u8]) {
    while !data.is_empty() {
        let n = fs.write_at(node, at, data).unwrap();
        at += n as u64;
        data = &data[n..];
    }
}

pub fn mkdir(fs: &mut Fs, dir: NodeId, text: &str) -> NodeId {
    fs.create(
        dir,
        Name::new(text).unwrap(),
        NewNode::Dir,
        &SetMetadata::new(),
    )
    .unwrap()
}

pub fn chain(fs: &mut Fs, node: NodeId) -> Vec<u32> {
    let mut clusters = Vec::new();
    fs.cluster_chain(node, |cluster| clusters.push(cluster))
        .unwrap();
    clusters
}

/// The names in the directory at `path`, in directory order.
pub fn names(fs: &mut Fs, path: &str) -> Vec<String> {
    let dir = fs.resolve(path).unwrap();
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut out = Vec::new();
    while fs
        .read_dir_entry(dir, &mut cursor, &mut buf)
        .unwrap()
        .is_some()
    {
        out.push(buf.as_name().unwrap().to_str().unwrap().to_owned());
    }
    fs.forget(dir);
    out
}

pub fn read(fs: &mut Fs, path: &str) -> Vec<u8> {
    fs.read_to_vec(path).unwrap()
}

/// Asserts that `check` finds nothing.
pub fn clean(fs: &mut Fs, what: &str) {
    let mut findings = Vec::new();
    let report =
        hadris_fat::exfat::sync::check_with(fs, &mut [0u8; 4096], |f| findings.push(f)).unwrap();
    assert!(report.is_clean(), "{what}: {findings:?}");
}

/// Formats a volume and fills it: nested directories, long and Unicode
/// names, a fragmented file, an empty file, attributes and a label.
pub fn build() -> Vec<u8> {
    let options = FormatOptions::new().with_label(VolumeLabel::new("Hadris").unwrap());
    let mut fs = formatted(8 << 20, options.with_cluster_size(4096));
    let root = fs.root();
    for (text, data) in [
        ("README.TXT", &b"hello exfat"[..]),
        ("lower.txt", b"lowercase"),
        ("\u{C9}t\u{E9} \u{1F600}.txt", b"unicode"),
        (&"n".repeat(255), b"longest name"),
        ("empty.dat", b""),
    ] {
        let node = write(&mut fs, root, text, data);
        fs.forget(node);
    }
    let frag = write(&mut fs, root, "frag.bin", &payload(100, 2));
    let spacer = write(&mut fs, root, "spacer.bin", &payload(100, 3));
    fs.forget(spacer);
    append(&mut fs, frag, 100, &payload(40_000, 4));
    fs.forget(frag);
    let nested = mkdir(&mut fs, root, "Nested Dir");
    let inner = mkdir(&mut fs, nested, "inner");
    for i in 0..300 {
        let text = format!("file number {i:03} with a longer name.txt");
        let node = write(&mut fs, inner, &text, text.as_bytes());
        fs.forget(node);
    }
    let deep = write(&mut fs, inner, "deep.bin", &payload(70_000, 5));
    fs.forget(deep);
    fs.forget(inner);
    fs.forget(nested);
    fs.sync().unwrap();
    image(fs)
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

/// Little-endian fields of a raw image.
pub fn le32(image: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(image[at..at + 4].try_into().unwrap())
}

pub fn le64(image: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(image[at..at + 8].try_into().unwrap())
}

/// Copies the main boot region of a volume with `sector`-byte sectors
/// over the backup and gives both a matching checksum.
pub fn seal_boot(image: &mut [u8], sector: usize) {
    let region = 12 * sector;
    let (main, rest) = image.split_at_mut(region);
    rest[..region].copy_from_slice(main);
    for base in [0, region] {
        let mut sum = 0u32;
        for (at, &byte) in image[base..base + 11 * sector].iter().enumerate() {
            if matches!(at, 106 | 107 | 112) {
                continue;
            }
            sum = sum.rotate_right(1).wrapping_add(byte as u32);
        }
        for word in image[base + 11 * sector..base + region].chunks_exact_mut(4) {
            word.copy_from_slice(&sum.to_le_bytes());
        }
    }
}

pub fn put32(image: &mut [u8], at: usize, value: u32) {
    image[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Where the structures of a raw image are.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub fat: usize,
    pub heap: usize,
    pub cluster: usize,
    pub count: u32,
    pub root: u32,
}

impl Geometry {
    pub fn of(image: &[u8]) -> Self {
        let sector = 1usize << image[108];
        Self {
            fat: le32(image, 80) as usize * sector,
            heap: le32(image, 88) as usize * sector,
            cluster: sector << image[109],
            count: le32(image, 92),
            root: le32(image, 96),
        }
    }

    pub fn at(&self, cluster: u32) -> usize {
        self.heap + (cluster as usize - 2) * self.cluster
    }

    pub fn fat(&self, image: &[u8], cluster: u32) -> u32 {
        le32(image, self.fat + cluster as usize * 4)
    }

    pub fn set_fat(&self, image: &mut [u8], cluster: u32, value: u32) {
        put32(image, self.fat + cluster as usize * 4, value);
    }

    /// The clusters of the chain at `first`.
    pub fn chain(&self, image: &[u8], first: u32) -> Vec<u32> {
        let mut out = vec![first];
        let mut cluster = first;
        while let next @ 2..=0xFFFF_FFF6 = self.fat(image, cluster) {
            out.push(next);
            cluster = next;
        }
        out
    }

    /// Offsets of every in-use entry of `kind` in the root directory.
    pub fn root_entries(&self, image: &[u8], kind: u8) -> Vec<usize> {
        let mut out = Vec::new();
        for cluster in self.chain(image, self.root) {
            let base = self.at(cluster);
            for at in (base..base + self.cluster).step_by(32) {
                if image[at] == kind {
                    out.push(at);
                }
            }
        }
        out
    }

    /// Sets or clears the bitmap bit of `cluster`.
    pub fn set_bit(&self, image: &mut [u8], cluster: u32, used: bool) {
        let entry = self.root_entries(image, 0x81)[0];
        let first = le32(image, entry + 20);
        let index = (cluster - 2) as usize;
        let chain = self.chain(image, first);
        let at = self.at(chain[index / 8 / self.cluster]) + index / 8 % self.cluster;
        if used {
            image[at] |= 1 << (index % 8);
        } else {
            image[at] &= !(1 << (index % 8));
        }
    }

    /// Moves cluster `index` of the chain at `first` to the free cluster
    /// `to`, relinking the chain and the bitmap.
    pub fn relocate(&self, image: &mut [u8], first: u32, index: usize, to: u32) {
        let chain = self.chain(image, first);
        let from = chain[index];
        assert_ne!(index, 0);
        let data = image[self.at(from)..self.at(from) + self.cluster].to_vec();
        image[self.at(to)..self.at(to) + self.cluster].copy_from_slice(&data);
        self.set_fat(image, chain[index - 1], to);
        let next = self.fat(image, from);
        self.set_fat(image, to, next);
        self.set_fat(image, from, 0);
        image[self.at(from)..self.at(from) + self.cluster].fill(0);
        self.set_bit(image, to, true);
        self.set_bit(image, from, false);
    }
}

/// A tool that checks an image without changing it.
#[derive(Debug, Clone, Copy)]
pub enum Tool {
    /// `fsck.exfat` from exfatprogs on the `PATH`.
    Local,
    /// `fsck.exfat` in the `hadris-exfatprogs` Docker image.
    Docker,
    /// macOS `fsck_exfat` on the image attached with `hdiutil`.
    MacOs,
}

fn available(tool: &Tool) -> bool {
    let probe = match tool {
        Tool::Local => Command::new("fsck.exfat").arg("-V").output(),
        Tool::Docker => Command::new("docker")
            .args(["image", "inspect", "hadris-exfatprogs"])
            .output(),
        Tool::MacOs => Command::new("hdiutil").arg("help").output(),
    };
    probe.is_ok_and(|out| out.status.success() || matches!(tool, Tool::Local))
        && (!matches!(tool, Tool::MacOs) || Path::new("/sbin/fsck_exfat").exists())
}

fn attach(path: &Path) -> Option<String> {
    let out = Command::new("hdiutil")
        .args([
            "attach",
            "-nomount",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
        ])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.split_whitespace().next()?.to_owned())
}

fn run(tool: &Tool, path: &Path) -> Option<(bool, String)> {
    let output = match tool {
        Tool::Local => Command::new("fsck.exfat")
            .arg("-n")
            .arg(path)
            .output()
            .ok()?,
        Tool::Docker => {
            let dir = path.parent()?;
            let name = path.file_name()?.to_str()?;
            Command::new("docker")
                .args(["run", "--rm", "-v"])
                .arg(format!("{}:/w", dir.display()))
                .args(["hadris-exfatprogs", "fsck.exfat", "-n"])
                .arg(format!("/w/{name}"))
                .output()
                .ok()?
        }
        Tool::MacOs => {
            let disk = attach(path)?;
            let raw = disk.replace("/dev/disk", "/dev/rdisk");
            let out = Command::new("/sbin/fsck_exfat")
                .args(["-n"])
                .arg(&raw)
                .output();
            let _ = Command::new("hdiutil").args(["detach", &disk]).output();
            out.ok()?
        }
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some((output.status.success(), text))
}

/// Runs every available native checker on `image` and fails on any
/// complaint. Returns how many ran. `hdiutil` attaches with 512-byte
/// blocks, so macOS `fsck_exfat` checks only volumes with 512-byte sectors. With `HADRIS_REQUIRE_EXTERNAL_TOOLS`
/// set, a run with no checker fails.
pub fn fsck(image: &[u8], label: &str) -> usize {
    let ran = fsck_with(image, label, &[Tool::Local, Tool::Docker, Tool::MacOs]);
    if std::env::var_os("HADRIS_REQUIRE_EXTERNAL_TOOLS").is_some() {
        assert!(ran > 0, "no exFAT checker is available for {label}");
    }
    ran
}

/// Runs the available checkers of `tools` on `image`, as [`fsck`] does.
pub fn fsck_with(image: &[u8], label: &str, tools: &[Tool]) -> usize {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.img");
    let mut ran = 0;
    for tool in tools {
        let attaches = !matches!(tool, Tool::MacOs) || image[108] == 9;
        if !attaches || !available(tool) {
            continue;
        }
        std::fs::write(&path, image).unwrap();
        let Some((ok, text)) = run(tool, &path) else {
            continue;
        };
        ran += 1;
        assert!(ok, "{tool:?} rejected {label}:\n{text}");
    }
    ran
}

impl Geometry {
    /// Offsets of every entry of the directory whose chain starts at
    /// `first`, in order.
    pub fn slots(&self, image: &[u8], first: u32) -> Vec<usize> {
        self.chain(image, first)
            .into_iter()
            .flat_map(|cluster| (self.at(cluster)..self.at(cluster) + self.cluster).step_by(32))
            .collect()
    }

    /// The entry offsets of the set named `name` in the directory at
    /// `first`.
    pub fn set(&self, image: &[u8], first: u32, name: &str) -> Vec<usize> {
        let slots = self.slots(image, first);
        let want: Vec<u16> = name.encode_utf16().collect();
        for (index, &at) in slots.iter().enumerate() {
            if image[at] != 0x85 {
                continue;
            }
            let count = 1 + image[at + 1] as usize;
            let set = slots[index..index + count].to_vec();
            let len = image[set[1] + 3] as usize;
            let units: Vec<u16> = (0..len)
                .map(|i| {
                    let entry = set[2 + i / 15] + 2 + (i % 15) * 2;
                    u16::from_le_bytes([image[entry], image[entry + 1]])
                })
                .collect();
            if units == want {
                return set;
            }
        }
        panic!("no entry set named {name}");
    }

    /// Recomputes the `SetChecksum` of the set at `set`.
    pub fn reseal(&self, image: &mut [u8], set: &[usize]) {
        let mut sum = 0u16;
        for (index, &at) in set.iter().enumerate() {
            for i in 0..32 {
                if index == 0 && (i == 2 || i == 3) {
                    continue;
                }
                sum = sum.rotate_right(1).wrapping_add(image[at + i] as u16);
            }
        }
        image[set[0] + 2..set[0] + 4].copy_from_slice(&sum.to_le_bytes());
    }

    /// Makes the chained allocation of the set at `set` contiguous
    /// (`NoFatChain`), clearing its FAT entries. The chain must already be
    /// contiguous.
    pub fn unchain(&self, image: &mut [u8], set: &[usize]) {
        let stream = set[1];
        let first = le32(image, stream + 20);
        let chain = self.chain(image, first);
        assert!(chain.windows(2).all(|pair| pair[1] == pair[0] + 1));
        for cluster in chain {
            self.set_fat(image, cluster, 0);
        }
        image[stream + 1] |= 0x02;
        self.reseal(image, set);
    }
}

/// Formats an image of `size` bytes with exfatprogs `mkfs.exfat`, from the
/// `PATH` or the `hadris-exfatprogs` Docker image.
pub fn mkfs_exfat(size: usize, label: &str) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mkfs.img");
    std::fs::write(&path, vec![0u8; size]).unwrap();
    let local = Command::new("mkfs.exfat")
        .args(["-L", label])
        .arg(&path)
        .output();
    let ok = match local {
        Ok(out) => out.status.success(),
        Err(_) => {
            if !available(&Tool::Docker) {
                return None;
            }
            Command::new("docker")
                .args(["run", "--rm", "-v"])
                .arg(format!("{}:/w", dir.path().display()))
                .args([
                    "hadris-exfatprogs",
                    "mkfs.exfat",
                    "-L",
                    label,
                    "/w/mkfs.img",
                ])
                .output()
                .ok()?
                .status
                .success()
        }
    };
    assert!(ok, "mkfs.exfat failed");
    Some(std::fs::read(&path).unwrap())
}

/// Formats an image of `size` bytes with macOS `newfs_exfat`.
pub fn newfs_exfat(size: usize, label: &str) -> Option<Vec<u8>> {
    if !available(&Tool::MacOs) || !Path::new("/sbin/newfs_exfat").exists() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("newfs.img");
    std::fs::write(&path, vec![0u8; size]).unwrap();
    let disk = attach(&path)?;
    let out = Command::new("/sbin/newfs_exfat")
        .args(["-v", label])
        .arg(disk.replace("/dev/disk", "/dev/rdisk"))
        .output();
    let _ = Command::new("hdiutil").args(["detach", &disk]).output();
    assert!(out.ok()?.status.success(), "newfs_exfat failed");
    Some(std::fs::read(&path).unwrap())
}

/// Mounts `image` with the macOS kernel driver, runs `f` on the mount
/// point, unmounts it and returns the image as the kernel left it. `None`
/// when `hdiutil` is not available.
pub fn with_macos_mount(image: &[u8], f: impl FnOnce(&Path)) -> Option<Vec<u8>> {
    if !available(&Tool::MacOs) {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kernel.img");
    let mount = dir.path().join("mnt");
    std::fs::create_dir(&mount).unwrap();
    std::fs::write(&path, image).unwrap();
    let out = Command::new("hdiutil")
        .args([
            "attach",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(&path)
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "hdiutil attach failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let disk = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .to_owned();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mount)));
    let detach = Command::new("hdiutil").args(["detach", &disk]).output();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    assert!(detach.ok()?.status.success(), "hdiutil detach failed");
    Some(std::fs::read(&path).unwrap())
}

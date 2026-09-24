#![allow(dead_code)]

use hadris_fs::MountOptions;
use std::path::Path;
use std::process::Command;

use hadris_fat::sync::{FatFs, format};
use hadris_fat::{Detail, FatKind, FormatOptions, VolumeLabel};
use hadris_fs::sync::FileSystem;
use hadris_fs::{
    Attributes, CheckReport, DirCursor, Finding, Location, Name, NodeId, SetAttr, Severity,
};

#[path = "paths.rs"]
pub mod paths;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockSize, MemDevice};
pub use paths::sync::{FsPaths, VolumePaths};

pub type Device = MemDevice<Vec<u8>>;
pub type Fs = FatFs<Device>;

#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub size: u64,
    pub kind: FatKind,
    /// The FAT sector size in bytes.
    pub sector: u32,
    pub block: u32,
}

pub const CASES: [Case; 5] = [
    Case {
        name: "fat12",
        size: 2 * 1024 * 1024,
        kind: FatKind::Fat12,
        sector: 512,
        block: 512,
    },
    Case {
        name: "fat16",
        size: 16 * 1024 * 1024,
        kind: FatKind::Fat16,
        sector: 512,
        block: 512,
    },
    Case {
        name: "fat32",
        size: 64 * 1024 * 1024,
        kind: FatKind::Fat32,
        sector: 512,
        block: 512,
    },
    Case {
        name: "fat12 with 4096-byte sectors on 512-byte blocks",
        size: 2 * 1024 * 1024,
        kind: FatKind::Fat12,
        sector: 4096,
        block: 512,
    },
    Case {
        name: "fat16 with 512-byte sectors on 4096-byte blocks",
        size: 16 * 1024 * 1024,
        kind: FatKind::Fat16,
        sector: 512,
        block: 4096,
    },
];

pub const LONG_NAME: &str = "A long file name.txt";
pub const UNICODE_NAME: &str = "\u{DC}n\u{EF}c\u{F6}d\u{E9} \u{F1}ame \u{1F600}.txt";
pub const INNER: &str = "/Nested Dir/inner";
pub const INNER_FILES: usize = 40;
/// A short entry whose name starts with `0xE5`, stored as `0x05`.
pub const KANJI_STORED: &[u8; 11] = b"XABC    TXT";
/// Read through the default `Cp437`, where `0xE5` is U+03C3.
pub const KANJI_NAME: &str = "\u{3C3}ABC.TXT";
/// Read through `Ascii`, which escapes `0xE5` as U+F7E5.
pub const KANJI_ASCII: &str = "\u{F7E5}ABC.TXT";

/// 204 UTF-16 units, 404 bytes of UTF-8.
pub fn huge_name() -> String {
    "\u{E9}".repeat(200) + ".txt"
}

pub fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
        .collect()
}

fn write(fs: &mut Fs, dir: NodeId, text: &str, data: &[u8]) -> NodeId {
    let node = fs.create(dir, Name::new(text), &SetAttr::new()).unwrap();
    append(fs, node, 0, data);
    node
}

fn append(fs: &mut Fs, node: NodeId, mut at: u64, mut data: &[u8]) {
    while !data.is_empty() {
        let n = fs.write(node, at, data).unwrap();
        at += n as u64;
        data = &data[n..];
    }
}

fn mkdir(fs: &mut Fs, dir: NodeId, text: &str) -> NodeId {
    fs.mkdir(dir, Name::new(text), &SetAttr::new()).unwrap()
}

/// The clusters of `node`'s chain.
pub fn chain<D: BlockDevice>(fs: &mut FatFs<D>, node: NodeId) -> Vec<u32> {
    let mut clusters = Vec::new();
    fs.cluster_chain(node, |cluster| clusters.push(cluster))
        .unwrap();
    clusters
}

/// Formats an image with `format` and fills it through `FatFs`: short,
/// lowercase, long, Unicode and over-255-byte names, nested and
/// multi-cluster directories, a fragmented file, attributes and a name whose
/// first byte is `0xE5`.
pub fn build(case: Case) -> Vec<u8> {
    let options = FormatOptions::new().with_label(VolumeLabel::new("HADRIS").unwrap());
    let mut fs = formatted(case, options);
    let root = fs.root();
    for (text, data) in [
        ("README.TXT", &b"hello fat"[..]),
        ("lower.txt", b"lowercase short name"),
        (LONG_NAME, &payload(5000, 1)),
        (UNICODE_NAME, b"unicode"),
        (&huge_name(), b"huge name"),
        ("empty.dat", b""),
    ] {
        let node = write(&mut fs, root, text, data);
        fs.forget(node, 1);
    }
    let hidden = write(&mut fs, root, "hidden.sys", b"h");
    fs.setattr(
        hidden,
        &SetAttr::new()
            .with_attributes(Attributes::HIDDEN | Attributes::SYSTEM | Attributes::READ_ONLY),
    )
    .unwrap();
    fs.forget(hidden, 1);

    let frag = write(&mut fs, root, "frag.bin", &payload(100, 2));
    let spacer = write(&mut fs, root, "spacer.bin", &payload(100, 3));
    assert_eq!(
        chain(&mut fs, spacer)[0],
        chain(&mut fs, frag)[0] + 1,
        "{}: frag.bin must be followed by spacer.bin",
        case.name
    );
    fs.forget(spacer, 1);
    append(&mut fs, frag, 100, &payload(40_000, 4));
    let clusters = chain(&mut fs, frag);
    assert!(
        clusters.windows(2).any(|pair| pair[1] != pair[0] + 1),
        "{}: frag.bin must be fragmented",
        case.name
    );
    fs.forget(frag, 1);

    let nested = mkdir(&mut fs, root, "Nested Dir");
    let sibling = write(&mut fs, nested, "sibling.txt", b"sibling");
    fs.forget(sibling, 1);
    let inner = mkdir(&mut fs, nested, "inner");
    let deep = write(&mut fs, inner, "deep.bin", &payload(70_000, 5));
    fs.forget(deep, 1);
    for i in 0..INNER_FILES - 1 {
        let text = format!("file number {i:02}.txt");
        let node = write(&mut fs, inner, &text, text.as_bytes());
        fs.forget(node, 1);
    }
    fs.forget(inner, 1);
    fs.forget(nested, 1);
    let kanji = write(&mut fs, root, "XABC.TXT", b"kanji");
    fs.forget(kanji, 1);
    fs.sync().unwrap();
    let mut image = fs.into_inner().into_inner();
    let at = image
        .chunks_exact(32)
        .position(|entry| &entry[..11] == KANJI_STORED)
        .expect("XABC.TXT entry")
        * 32;
    image[at] = 0x05;
    image
}

/// Formats a device for `case` with `options`, the case's kind and sector
/// size added.
pub fn formatted(case: Case, options: FormatOptions) -> Fs {
    let dev = device(case, vec![0u8; case.size as usize]);
    let options = options.with_kind(case.kind).with_sector_size(case.sector);
    FatFs::mount(
        format(dev, options).unwrap().into_inner(),
        MountOptions::new(),
    )
    .unwrap()
}

/// A freshly formatted image with nothing on it.
pub fn blank(case: Case) -> Vec<u8> {
    formatted(case, FormatOptions::new())
        .into_inner()
        .into_inner()
}

/// Mounts a copy of `image` afresh, to read back what was written.
pub fn mount(case: Case, image: &[u8]) -> Fs {
    FatFs::mount(device(case, image.to_vec()), MountOptions::new()).unwrap()
}

/// The names in the directory at `path`, in directory order.
pub fn names(fs: &mut Fs, path: &str) -> Vec<String> {
    let dir = fs.resolve_path(path).unwrap();
    let mut cursor = DirCursor::START;
    let mut out = Vec::new();
    while let Some(entry) = fs.readdir(dir, cursor).unwrap() {
        out.push(entry.name().to_str().unwrap().to_owned());
        cursor = entry.next_cursor();
    }
    fs.forget(dir, 1);
    out
}

/// The contents of the file at `path`.
pub fn read(fs: &mut Fs, path: &str) -> Vec<u8> {
    fs.read_to_vec(path).unwrap()
}

/// Mounts `dev` with a heap node table.
pub fn mount_dev(dev: Device) -> Fs {
    FatFs::mount(dev, MountOptions::new()).unwrap()
}

pub fn device(case: Case, image: Vec<u8>) -> Device {
    MemDevice::new(image, BlockSize::new(case.block).unwrap())
}

/// A finding copied out of the `check` callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub detail: Detail,
    pub severity: Severity,
    pub location: Option<Location>,
    pub path: Option<String>,
    pub message: &'static str,
}

impl Found {
    pub fn new(finding: &Finding<'_>) -> Self {
        Self {
            detail: Detail::from_code(finding.detail()).unwrap(),
            severity: finding.severity(),
            location: finding.location(),
            path: finding
                .path()
                .map(|path| String::from_utf8_lossy(path).into_owned()),
            message: finding.message(),
        }
    }
}

/// Checks `dev` with `scratch` bytes of scratch space and returns every
/// finding.
pub fn check_dev<D: BlockDevice>(dev: &mut D, scratch: usize) -> (CheckReport, Vec<Found>) {
    let mut found = Vec::new();
    let mut scratch = vec![0u8; scratch];
    let report =
        hadris_fat::sync::check(dev, &mut scratch, |finding| found.push(Found::new(finding)))
            .unwrap();
    assert_eq!(report.findings() as usize, found.len());
    (report, found)
}

/// The free clusters of the volume on `dev`, counted by scanning its FAT.
pub fn scan_free<D: BlockDevice>(dev: &mut D) -> u32 {
    use hadris_fat::raw::io::{BlockBuf, Fat, sync as rawio};
    let mut block = BlockBuf::<[u8; 4096]>::new(dev.block_size().get() as usize).unwrap();
    let geo = rawio::read_geometry(dev, &mut block).unwrap();
    rawio::count_free(dev, &mut block, &mut Fat::new(geo)).unwrap()
}

/// Asserts that `check` finds nothing on `image`, and returns the free
/// clusters its FAT holds.
pub fn assert_checks_clean(case: Case, image: &[u8], what: &str) -> u32 {
    let mut dev = device(case, image.to_vec());
    let (_, found) = check_dev(&mut dev, 8192);
    assert_eq!(found, [], "{what}");
    scan_free(&mut dev)
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

/// A tool that checks an image file without changing it.
struct Fsck {
    program: &'static str,
    args: &'static [&'static str],
}

const FSCKS: [Fsck; 2] = [
    Fsck {
        program: "fsck.fat",
        args: &["-n", "-V"],
    },
    Fsck {
        program: "fsck_msdos",
        args: &["-n"],
    },
];

/// Runs every installed `fsck` on `image` and fails on any complaint.
/// Returns how many ran.
pub fn fsck(image: &[u8], label: &str) -> usize {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.img");
    std::fs::write(&path, image).unwrap();
    let mut ran = 0;
    for tool in FSCKS {
        let Some(output) = run(tool.program, tool.args, &path) else {
            continue;
        };
        ran += 1;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "{} rejected {label}:\n{text}",
            tool.program
        );
        std::fs::write(&path, image).unwrap();
    }
    ran
}

fn run(program: &str, args: &[&str], path: &Path) -> Option<std::process::Output> {
    Command::new(program).args(args).arg(path).output().ok()
}

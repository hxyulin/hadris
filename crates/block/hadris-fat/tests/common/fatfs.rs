#![allow(dead_code)]

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter, SectorSize};
use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::write::FileWriter;
use hadris_fat::{FatDir, FatKind, FatVolume, FatVolumeWriteExt, FileEntry};
use hadris_io::StdIo;
use hadris_storage::{BlockSize, MemDevice};

pub type Image = StdIo<Cursor<Vec<u8>>>;
pub type Device = MemDevice<Vec<u8>>;

#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub size: u64,
    pub selection: FatTypeSelection,
    pub kind: FatKind,
    pub sector: SectorSize,
    pub block: u32,
}

pub const CASES: [Case; 5] = [
    Case {
        name: "fat12",
        size: 2 * 1024 * 1024,
        selection: FatTypeSelection::Fat12,
        kind: FatKind::Fat12,
        sector: SectorSize::S512,
        block: 512,
    },
    Case {
        name: "fat16",
        size: 16 * 1024 * 1024,
        selection: FatTypeSelection::Fat16,
        kind: FatKind::Fat16,
        sector: SectorSize::S512,
        block: 512,
    },
    Case {
        name: "fat32",
        size: 64 * 1024 * 1024,
        selection: FatTypeSelection::Fat32,
        kind: FatKind::Fat32,
        sector: SectorSize::S512,
        block: 512,
    },
    Case {
        name: "fat12 with 4096-byte sectors on 512-byte blocks",
        size: 2 * 1024 * 1024,
        selection: FatTypeSelection::Fat12,
        kind: FatKind::Fat12,
        sector: SectorSize::S4096,
        block: 512,
    },
    Case {
        name: "fat16 with 512-byte sectors on 4096-byte blocks",
        size: 16 * 1024 * 1024,
        selection: FatTypeSelection::Fat16,
        kind: FatKind::Fat16,
        sector: SectorSize::S512,
        block: 4096,
    },
];

pub const LONG_NAME: &str = "A long file name.txt";
pub const UNICODE_NAME: &str = "\u{DC}n\u{EF}c\u{F6}d\u{E9} \u{F1}ame \u{1F600}.txt";
pub const INNER: &str = "/Nested Dir/inner";
pub const INNER_FILES: usize = 40;
/// A short entry whose name starts with `0xE5`, stored as `0x05`.
pub const KANJI_STORED: &[u8; 11] = b"XABC    TXT";
pub const KANJI_NAME: &str = "\u{FFFD}ABC.TXT";

/// 204 UTF-16 units, 404 bytes of UTF-8.
pub fn huge_name() -> String {
    "\u{E9}".repeat(200) + ".txt"
}

pub fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
        .collect()
}

fn write(volume: &FatVolume<Image>, dir: &FatDir<'_, Image>, name: &str, data: &[u8]) -> FileEntry {
    let file = volume.create_file(dir, name).unwrap();
    let mut writer = volume.write_file(&file).unwrap();
    if !data.is_empty() {
        writer.write(data).unwrap();
    }
    writer.finish().unwrap();
    dir.find(name).unwrap().unwrap()
}

/// Formats an image with the V2 formatter and fills it with the V2 writer:
/// short, lowercase, long, Unicode and over-255-byte names, nested and
/// multi-cluster directories, a fragmented file, attributes and a name whose
/// first byte is `0xE5`.
pub fn build(case: Case) -> Vec<u8> {
    let options = FatFormatOptions::new(case.size)
        .fat_type(case.selection)
        .sector_size(case.sector)
        .volume_label("HADRIS");
    let image = StdIo::new(Cursor::new(vec![0u8; case.size as usize]));
    let volume = FatVolumeFormatter::format(image, options).unwrap();
    {
        let root = volume.root_dir();
        write(&volume, &root, "README.TXT", b"hello fat");
        write(&volume, &root, "lower.txt", b"lowercase short name");
        write(&volume, &root, LONG_NAME, &payload(5000, 1));
        write(&volume, &root, UNICODE_NAME, b"unicode");
        write(&volume, &root, &huge_name(), b"huge name");
        write(&volume, &root, "empty.dat", b"");
        let hidden = write(&volume, &root, "hidden.sys", b"h");
        volume
            .set_attributes(
                &hidden,
                DirEntryAttrFlags::HIDDEN
                    | DirEntryAttrFlags::SYSTEM
                    | DirEntryAttrFlags::READ_ONLY,
            )
            .unwrap();

        let frag = write(&volume, &root, "frag.bin", &payload(100, 2));
        let spacer = write(&volume, &root, "spacer.bin", &payload(100, 3));
        assert_eq!(
            spacer.cluster().0,
            frag.cluster().0 + 1,
            "{}: frag.bin must be followed by spacer.bin",
            case.name
        );
        let mut writer = FileWriter::new_append(&volume, &frag).unwrap();
        writer.write(&payload(40_000, 4)).unwrap();
        writer.finish().unwrap();

        let nested = volume.create_dir(&root, "Nested Dir").unwrap();
        write(&volume, &nested, "sibling.txt", b"sibling");
        let inner = volume.create_dir(&nested, "inner").unwrap();
        write(&volume, &inner, "deep.bin", &payload(70_000, 5));
        for i in 0..INNER_FILES - 1 {
            let name = format!("file number {i:02}.txt");
            write(&volume, &inner, &name, name.as_bytes());
        }
        write(&volume, &root, "XABC.TXT", b"kanji");
    }
    volume.sync().unwrap();
    let mut image = volume.into_inner().into_inner().into_inner();
    let at = image
        .chunks_exact(32)
        .position(|entry| &entry[..11] == KANJI_STORED)
        .expect("XABC.TXT entry")
        * 32;
    image[at] = 0x05;
    image
}

/// A freshly formatted image with nothing on it.
pub fn blank(case: Case) -> Vec<u8> {
    let options = FatFormatOptions::new(case.size)
        .fat_type(case.selection)
        .sector_size(case.sector);
    let image = StdIo::new(Cursor::new(vec![0u8; case.size as usize]));
    let volume = FatVolumeFormatter::format(image, options).unwrap();
    volume.sync().unwrap();
    volume.into_inner().into_inner().into_inner()
}

pub fn device(case: Case, image: Vec<u8>) -> Device {
    MemDevice::new(image, BlockSize::new(case.block).unwrap())
}

pub fn open_v2(image: &[u8]) -> FatVolume<Image> {
    FatVolume::open(StdIo::new(Cursor::new(image.to_vec()))).unwrap()
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

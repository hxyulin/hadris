use std::collections::BTreeMap;
use std::convert::Infallible;
use std::hint::black_box;

use hadris_fat::exfat::{ExFatOptions, sync::ExFatFs};
use hadris_fat::{FatKind, FatOptions, sync::FatFs};
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, DirCursor, ErrorKind, MountOptions, Name, Node, OpenMode, Tree};
use hadris_iso::{IsoOptions, sync::IsoFs};
use hadris_storage::{BlockSize, MemDevice};
use hadris_tests::harness::performance::{Counted, Measurement};
use hadris_tests::harness::{EntryData, Workspace, write_report};
use hadris_udf::{UdfOptions, sync::UdfFs};

type Device<'a> = Counted<MemDevice<&'a [u8]>>;
const PAYLOAD: &str = "PAYLOAD.BIN";

#[derive(Clone, Copy)]
enum Format {
    Fat(FatKind),
    ExFat,
    Iso,
    Udf,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::Fat(FatKind::Fat12) => "fat12",
            Self::Fat(FatKind::Fat16) => "fat16",
            Self::Fat(FatKind::Fat32) => "fat32",
            Self::Fat(_) => panic!("unsupported FAT kind"),
            Self::ExFat => "exfat",
            Self::Iso => "iso",
            Self::Udf => "udf",
        }
    }

    fn image(self, tree: &Tree) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Fat(kind) => {
                let size = match kind {
                    FatKind::Fat12 => 2 << 20,
                    FatKind::Fat16 => 16 << 20,
                    FatKind::Fat32 => 64 << 20,
                    _ => panic!("unsupported FAT kind"),
                };
                hadris_fat::sync::write(
                    &mut out,
                    tree,
                    &FatOptions::new()
                        .with_kind(kind)
                        .with_label(hadris_fat::VolumeLabel::new("PERF").unwrap())
                        .with_size(size),
                )
                .unwrap();
            }
            Self::ExFat => {
                hadris_fat::exfat::sync::write(
                    &mut out,
                    tree,
                    &ExFatOptions::new()
                        .with_label(hadris_fat::exfat::VolumeLabel::new("PERF").unwrap())
                        .with_size(16 << 20),
                )
                .unwrap();
            }
            Self::Iso => {
                hadris_iso::sync::write(&mut out, tree, &IsoOptions::default()).unwrap();
            }
            Self::Udf => {
                hadris_udf::sync::write(&mut out, tree, &UdfOptions::default()).unwrap();
            }
        }
        out.resize(out.len().next_multiple_of(512), 0);
        out
    }
}

fn measure<'a, F: FileSystem<DeviceError = Infallible>>(
    name: &str,
    bytes: &'a [u8],
    payload: &[u8],
    samples: usize,
    mount: impl Fn(Device<'a>) -> F,
    rows: &mut Vec<String>,
) {
    for workload in [
        "mount",
        "list",
        "lookup-last",
        "lookup-miss",
        "stat",
        "read-4k",
        "read-64k",
        "read-scattered",
    ] {
        for sample in 0..=samples {
            let (dev, counts) = Counted::new(MemDevice::new(bytes, BlockSize::new(512).unwrap()));
            let (mut fs, mounted) = Measurement::run(&counts, || mount(dev));
            let measured = if workload == "mount" {
                mounted
            } else {
                let root = fs.root();
                let node = if workload == "stat" || workload.starts_with("read-") {
                    fs.lookup(root, Name::new(PAYLOAD)).unwrap()
                } else {
                    root
                };
                let opened = workload.starts_with("read-");
                if opened {
                    fs.open(node, OpenMode::Read).unwrap();
                }
                let mut buf = vec![0; if workload == "read-64k" { 65536 } else { 4096 }];
                let offsets = [65536, 0, 122880, 4096, 98304];
                let mut chunks = [[0u8; 4096]; 5];
                let mut lengths = [0; 5];
                let mut names = Vec::with_capacity(33);
                let (observed, measured) = Measurement::run(&counts, || match workload {
                    "list" => {
                        let mut cursor = DirCursor::START;
                        let mut count = 0;
                        while let Some(entry) = fs.readdir(root, cursor).unwrap() {
                            cursor = entry.next_cursor();
                            names.push(black_box(entry));
                            count += 1;
                        }
                        count
                    }
                    "lookup-last" => {
                        let found = fs.lookup(root, Name::new(PAYLOAD)).unwrap();
                        fs.forget(found, 1);
                        1
                    }
                    "lookup-miss" => {
                        assert_eq!(
                            fs.lookup(root, Name::new("MISSING.BIN"))
                                .unwrap_err()
                                .kind(),
                            ErrorKind::NotFound
                        );
                        1
                    }
                    "stat" => black_box(fs.stat(node).unwrap()).len() as usize,
                    "read-scattered" => {
                        for ((offset, chunk), length) in
                            offsets.iter().zip(&mut chunks).zip(&mut lengths)
                        {
                            *length = fs.read(node, *offset, chunk).unwrap();
                        }
                        5
                    }
                    _ => fs.read(node, 0, &mut buf).unwrap(),
                });
                match workload {
                    "list" => {
                        assert_eq!(observed, 33);
                        let mut actual: Vec<_> = names
                            .iter()
                            .map(|entry| {
                                let name = std::str::from_utf8(entry.name().as_bytes()).unwrap();
                                name.strip_suffix(";1").unwrap_or(name).to_string()
                            })
                            .collect();
                        actual.sort();
                        let mut expected: Vec<_> =
                            (0..32).map(|index| format!("F{index:07}.TXT")).collect();
                        expected.push(PAYLOAD.to_string());
                        expected.sort();
                        assert_eq!(actual, expected);
                    }
                    "stat" => assert_eq!(observed, payload.len()),
                    "read-scattered" => {
                        assert_eq!(observed, 5);
                        for ((offset, bytes), n) in offsets.iter().zip(chunks).zip(lengths) {
                            assert_eq!(n, bytes.len());
                            let offset = *offset as usize;
                            assert_eq!(bytes, payload[offset..offset + n]);
                        }
                    }
                    name if name.starts_with("read-") => {
                        assert_eq!(observed, buf.len());
                        assert_eq!(buf, payload[..observed]);
                    }
                    _ => assert_eq!(observed, 1),
                }
                if opened {
                    fs.close(node).unwrap();
                }
                if node != root {
                    fs.forget(node, 1);
                }
                measured
            };
            assert_eq!(measured.io.failures, 0);
            assert_eq!(measured.io.write_calls, 0);
            if sample != 0 {
                rows.push(measured.csv_row(name, workload, sample));
            }
        }
    }
    let (dev, _) = Counted::new(MemDevice::new(bytes, BlockSize::new(512).unwrap()));
    let mut fs = mount(dev);
    let node = fs.lookup(fs.root(), Name::new(PAYLOAD)).unwrap();
    fs.open(node, OpenMode::Read).unwrap();
    let mut actual = vec![0; payload.len()];
    let mut at = 0;
    while at < actual.len() {
        let n = fs.read(node, at as u64, &mut actual[at..]).unwrap();
        assert!(n > 0);
        at += n;
    }
    assert_eq!(actual, payload);
    fs.close(node).unwrap();
    fs.forget(node, 1);
}

fn main() {
    let samples = std::env::var("HADRIS_TESTS_PERF_SAMPLES").map_or(21, |text| {
        text.parse::<usize>().expect("positive sample count")
    });
    assert!(samples > 0);
    let filter = std::env::var("HADRIS_TESTS_PERF_FILTER").unwrap_or_default();
    let payload: Vec<u8> = (0..131072).map(|i| (i * 17 + i / 251) as u8).collect();
    let mut tree = Tree::new();
    for index in 0..32 {
        tree.insert(
            format!("F{index:07}.TXT"),
            Node::file(Content::bytes(b"fixture".as_slice())),
        )
        .unwrap();
    }
    tree.insert(PAYLOAD, Node::file(Content::bytes(payload.clone())))
        .unwrap();
    let mut rows = vec![Measurement::CSV_HEADER.to_string()];
    for format in [
        Format::Fat(FatKind::Fat12),
        Format::Fat(FatKind::Fat16),
        Format::Fat(FatKind::Fat32),
        Format::ExFat,
        Format::Iso,
        Format::Udf,
    ] {
        let name = format.name();
        if !name.contains(&filter) {
            continue;
        }
        let bytes = format.image(&tree);
        verify_oracle(format, &bytes, &payload);
        match format {
            Format::Fat(_) => measure(
                name,
                &bytes,
                &payload,
                samples,
                |dev| FatFs::mount(dev, MountOptions::new()).unwrap(),
                &mut rows,
            ),
            Format::ExFat => measure(
                name,
                &bytes,
                &payload,
                samples,
                |dev| ExFatFs::mount(dev, MountOptions::new()).unwrap(),
                &mut rows,
            ),
            Format::Iso => measure(
                name,
                &bytes,
                &payload,
                samples,
                |dev| IsoFs::mount(dev, MountOptions::new()).unwrap(),
                &mut rows,
            ),
            Format::Udf => measure(
                name,
                &bytes,
                &payload,
                samples,
                |dev| UdfFs::mount(dev, MountOptions::new()).unwrap(),
                &mut rows,
            ),
        }
    }
    assert!(rows.len() > 1, "filter matched no formats");
    write_report("performance", "v3.csv", &rows.join("\n")).unwrap();
}

fn verify_oracle(format: Format, bytes: &[u8], payload: &[u8]) {
    let mut expected = BTreeMap::new();
    for index in 0..32 {
        expected.insert(
            format!("/F{index:07}.TXT"),
            EntryData::File(b"fixture".to_vec()),
        );
    }
    expected.insert(format!("/{PAYLOAD}"), EntryData::File(payload.to_vec()));
    let actual = match format {
        Format::Fat(kind) => {
            let workspace = Workspace::new("performance", "oracle").unwrap();
            let path = workspace.path.join("fixture.img");
            std::fs::write(&path, bytes).unwrap();
            let bits = match kind {
                FatKind::Fat12 => 12,
                FatKind::Fat16 => 16,
                FatKind::Fat32 => 32,
                _ => panic!("unsupported FAT kind"),
            };
            hadris_tests::fat::spec::snapshot(&path, bits)
                .unwrap()
                .entries
                .into_iter()
                .map(|(path, entry)| (path, entry.data))
                .collect()
        }
        Format::ExFat => {
            let workspace = Workspace::new("performance", "oracle").unwrap();
            let path = workspace.path.join("fixture.img");
            std::fs::write(&path, bytes).unwrap();
            hadris_tests::exfat::spec::snapshot(&path)
                .unwrap()
                .entries
                .into_iter()
                .map(|(path, entry)| (path, entry.data))
                .collect()
        }
        Format::Iso => hadris_tests::iso::spec::snapshot(bytes).unwrap().entries,
        Format::Udf => return,
    };
    assert_eq!(actual, expected);
}

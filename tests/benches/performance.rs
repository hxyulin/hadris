#[path = "support/fixture.rs"]
mod fixture;

use fixture::{Fixture, PAYLOAD};
use std::collections::BTreeMap;
use std::hint::black_box;

use hadris_fat::exfat::{ExFatOptions, sync::ExFatFs};
use hadris_fat::{FatKind, FatOptions, sync::FatFs};
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, MountOptions, Name, OpenMode, Tree};
use hadris_iso::{IsoOptions, sync::IsoFs};
use hadris_storage::host::FileDevice;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockSize, MemDevice};
use hadris_tests::harness::performance::{Counted, Measurement};
use hadris_tests::harness::resources::run_with_peak_rss;
use hadris_tests::harness::{EntryData, Workspace, write_report};
use hadris_udf::{UdfOptions, sync::UdfFs};

const WORKLOADS: &[&str] = &[
    "mount",
    "list",
    "lookup-last",
    "lookup-miss",
    "lookup-last-warm",
    "lookup-batch",
    "stat",
    "read-4k",
    "read-64k",
    "read-scattered",
    "read-full",
];

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

fn measure<D: BlockDevice, F: FileSystem>(
    name: &str,
    device: impl Fn() -> D,
    payload: &[u8],
    samples: usize,
    file_count: usize,
    mount: impl Fn(Counted<D>) -> F,
    rows: &mut Vec<String>,
) {
    let selected = std::env::var("HADRIS_TESTS_PERF_WORKLOAD").unwrap_or_default();
    for &workload in WORKLOADS {
        if !selected.is_empty() && workload != selected {
            continue;
        }
        for sample in 0..=samples {
            let (dev, counts) = Counted::new(device());
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
                if workload == "lookup-last-warm" {
                    let found = fs.lookup(root, Name::new(PAYLOAD)).unwrap();
                    fs.forget(found, 1);
                }
                let lookup_names: Vec<_> = if workload == "lookup-batch" {
                    (0..file_count).map(|i| format!("F{i:07}.TXT")).collect()
                } else {
                    Vec::new()
                };
                let size = match workload {
                    "read-full" => payload.len(),
                    "read-64k" => 65536,
                    "read-4k" => 4096,
                    _ => 0,
                };
                let mut buf = vec![0; size];
                let offsets = [65536, 0, 122880, 4096, 98304];
                let mut chunks = [[0u8; 4096]; 5];
                let mut lengths = [0; 5];
                let mut names = Vec::with_capacity(if workload == "list" {
                    file_count + 1
                } else {
                    0
                });
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
                    "lookup-last" | "lookup-last-warm" => {
                        let found = fs.lookup(root, Name::new(PAYLOAD)).unwrap();
                        fs.forget(found, 1);
                        1
                    }
                    "lookup-batch" => {
                        for name in &lookup_names {
                            let found = fs.lookup(root, Name::new(name)).unwrap();
                            fs.forget(found, 1);
                        }
                        lookup_names.len()
                    }
                    "read-full" => {
                        let mut at = 0;
                        while at < buf.len() {
                            let n = fs.read(node, at as u64, &mut buf[at..]).unwrap();
                            assert!(n > 0, "premature EOF");
                            at += n;
                        }
                        at
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
                        assert_eq!(observed, file_count + 1);
                        let mut actual: Vec<_> = names
                            .iter()
                            .map(|entry| {
                                let name = std::str::from_utf8(entry.name().as_bytes()).unwrap();
                                name.strip_suffix(";1").unwrap_or(name).to_string()
                            })
                            .collect();
                        actual.sort();
                        let mut expected: Vec<_> = (0..file_count)
                            .map(|index| format!("F{index:07}.TXT"))
                            .collect();
                        expected.push(PAYLOAD.to_string());
                        expected.sort();
                        assert_eq!(actual, expected);
                    }
                    "stat" => assert_eq!(observed, payload.len()),
                    "lookup-batch" => assert_eq!(observed, file_count),
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
}

fn run_format<D: BlockDevice>(
    format: Format,
    device: impl Fn() -> D,
    payload: &[u8],
    samples: usize,
    file_count: usize,
    rows: &mut Vec<String>,
) {
    let cache = std::env::var("HADRIS_TESTS_PERF_CACHE").unwrap_or_else(|_| "default".into());
    match format {
        Format::Fat(_) => measure(
            format.name(),
            device,
            payload,
            samples,
            file_count,
            |dev| {
                let fs = FatFs::mount(dev, MountOptions::new()).unwrap();
                let options = match cache.as_str() {
                    "default" => hadris_fat::CacheOptions::new(),
                    "none" => hadris_fat::CacheOptions::new()
                        .with_blocks(0)
                        .with_chain_positions(0),
                    "index" => {
                        hadris_fat::CacheOptions::new().with_directory_entries(file_count + 1)
                    }
                    _ => panic!("unsupported FAT cache configuration"),
                };
                fs.with_cache(options)
            },
            rows,
        ),
        Format::ExFat => measure(
            format.name(),
            device,
            payload,
            samples,
            file_count,
            |dev| ExFatFs::mount(dev, MountOptions::new()).unwrap(),
            rows,
        ),
        Format::Iso => measure(
            format.name(),
            device,
            payload,
            samples,
            file_count,
            |dev| {
                let fs = IsoFs::mount(dev, MountOptions::new()).unwrap();
                match cache.as_str() {
                    "default" | "none" => fs,
                    "index" => {
                        fs.with_cache(hadris_iso::CacheOptions::new().with_records(file_count + 1))
                    }
                    _ => panic!("unsupported ISO cache configuration"),
                }
            },
            rows,
        ),
        Format::Udf => measure(
            format.name(),
            device,
            payload,
            samples,
            file_count,
            |dev| UdfFs::mount(dev, MountOptions::new()).unwrap(),
            rows,
        ),
    }
}

fn formats() -> [Format; 6] {
    [
        Format::Fat(FatKind::Fat12),
        Format::Fat(FatKind::Fat16),
        Format::Fat(FatKind::Fat32),
        Format::ExFat,
        Format::Iso,
        Format::Udf,
    ]
}

fn main() {
    let samples = std::env::var("HADRIS_TESTS_PERF_SAMPLES").map_or(21, |text| {
        text.parse::<usize>().expect("positive sample count")
    });
    assert!(samples > 0);
    let filter = std::env::var("HADRIS_TESTS_PERF_FILTER").unwrap_or_default();
    let file_count =
        std::env::var("HADRIS_TESTS_PERF_FILES").map_or(32, |text| text.parse::<usize>().unwrap());
    let backend = std::env::var("HADRIS_TESTS_PERF_BACKEND").unwrap_or_else(|_| "memory".into());
    let cache = std::env::var("HADRIS_TESTS_PERF_CACHE").unwrap_or_else(|_| "default".into());
    assert!(["default", "none", "index"].contains(&cache.as_str()));
    assert!(["memory", "file"].contains(&backend.as_str()));
    if let Some(image) = std::env::var_os("HADRIS_TESTS_PERF_IMAGE") {
        let format = formats()
            .into_iter()
            .find(|f| f.name() == filter)
            .expect("worker needs exact format");
        let payload = fixture::payload();
        let mut rows = Vec::new();
        run_format(
            format,
            || FileDevice::open(&image).unwrap(),
            &payload,
            1,
            file_count,
            &mut rows,
        );
        assert_eq!(rows.len(), 1, "worker needs exactly one workload");
        println!("{}", rows[0]);
        return;
    }
    let Fixture {
        tree,
        payload,
        entries,
    } = Fixture::new();
    let mut rows = vec![format!(
        "{},backend,cache,file_count,peak_rss_bytes",
        Measurement::CSV_HEADER
    )];
    for format in formats() {
        if !format.name().contains(&filter) {
            continue;
        }
        if matches!(format, Format::Fat(FatKind::Fat12 | FatKind::Fat16)) {
            assert!(
                file_count <= 128,
                "use fat32/iso/udf/exfat filters for large directories"
            );
        }
        if cache != "default" {
            assert!(
                matches!(format, Format::Fat(_) | Format::Iso),
                "cache comparison supports FAT/ISO only"
            );
        }
        let bytes = format.image(&tree);
        verify_oracle(format, &bytes, &entries);
        if backend == "memory" {
            let mut measured = Vec::new();
            run_format(
                format,
                || MemDevice::new(bytes.as_slice(), BlockSize::new(512).unwrap()),
                &payload,
                samples,
                file_count,
                &mut measured,
            );
            rows.extend(
                measured
                    .into_iter()
                    .map(|row| format!("{row},memory,{cache},{file_count},")),
            );
        } else {
            let workspace = Workspace::new("performance", "resources").unwrap();
            let image = workspace.path.join("fixture.img");
            std::fs::write(&image, &bytes).unwrap();
            for &workload in WORKLOADS {
                let selected = std::env::var("HADRIS_TESTS_PERF_WORKLOAD").unwrap_or_default();
                if !selected.is_empty() && selected != workload {
                    continue;
                }
                for sample in 1..=samples {
                    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                    command
                        .env("HADRIS_TESTS_PERF_IMAGE", &image)
                        .env("HADRIS_TESTS_PERF_FILTER", format.name())
                        .env("HADRIS_TESTS_PERF_WORKLOAD", workload);
                    let (output, rss) = run_with_peak_rss(&mut command).unwrap();
                    let row = String::from_utf8(output.stdout).unwrap();
                    let fields: Vec<_> = row.trim().split(',').collect();
                    assert_eq!(fields.len(), 11, "invalid worker row");
                    let mut fields: Vec<_> = fields.iter().map(|s| s.to_string()).collect();
                    fields[2] = sample.to_string();
                    rows.push(format!(
                        "{},file,{cache},{file_count},{rss}",
                        fields.join(",")
                    ));
                }
            }
        }
    }
    assert!(rows.len() > 1, "filter matched no formats or workloads");
    write_report("performance", "v3.csv", &rows.join("\n")).unwrap();
    let mut metadata = vec![
        format!("compiler: {}", env!("HADRIS_TESTS_COMPILER")),
        format!("profile: {}", env!("HADRIS_TESTS_PROFILE")),
        format!("host: {}/{}", std::env::consts::OS, std::env::consts::ARCH),
    ];
    for (key, args) in [
        ("revision", vec!["rev-parse", "HEAD"]),
        ("dirty", vec!["status", "--porcelain"]),
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        metadata.push(format!(
            "{key}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        ));
    }
    metadata.push(format!("backend: {backend}\ncache: {cache}\nfiles: {file_count}\nsamples: {samples}\nfixture: flat root, 128 KiB payload\nRSS: isolated process peak, includes mount, warm-up and verification; fixture creation excluded\nfile backend: OS page cache uncontrolled; not physical cold-disk latency\ncache index: FAT directory entries and ISO records; different policies"));
    write_report("performance", "metadata.txt", &metadata.join("\n")).unwrap();
}

fn verify_oracle(format: Format, bytes: &[u8], expected: &BTreeMap<String, EntryData>) {
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
    assert_eq!(&actual, expected);
}

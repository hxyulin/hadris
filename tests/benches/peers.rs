#[path = "support/fixture.rs"]
mod fixture;
#[path = "support/profile.rs"]
mod profile;
#[path = "support/worker.rs"]
mod worker;

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use fixture::Fixture;
use hadris_fat::{FatKind, FatOptions, VolumeLabel, sync::FatFs};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_iso::{IsoId, IsoOptions, sync::IsoFs};
use hadris_storage::host::FileDevice;
use hadris_tests::fat::{FAT_CASES, FatCase, LABEL, fatfs as rustfatfs, mtools};
use hadris_tests::harness::files::{entries, read_node};
use hadris_tests::harness::performance::{Counted, CountedStream, IoCounts};
use hadris_tests::harness::tree::{snapshot_host, write_host_tree};
use hadris_tests::harness::{Workspace, require_or_skip, run_command, write_report};
use hadris_tests::iso::{spec as iso_spec, xorriso};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Peer {
    Hadris,
    RustFatfs,
    RustFatfsBuffered,
    Mtools,
    ChanFatfs,
    Xorriso,
    Mkisofs(&'static str),
    Bsdtar,
}
impl Peer {
    fn name(self) -> &'static str {
        match self {
            Self::Hadris => "hadris",
            Self::RustFatfs => "rust-fatfs/unbuffered",
            Self::RustFatfsBuffered => "rust-fatfs/buffered",
            Self::Mtools => "dosfstools+mtools",
            Self::ChanFatfs => "chan-fatfs/helper",
            Self::Xorriso => "xorriso/libisofs",
            Self::Mkisofs(program) => program,
            Self::Bsdtar => "bsdtar/libarchive",
        }
    }
    fn available(self) -> bool {
        match self {
            Self::Hadris | Self::RustFatfs | Self::RustFatfsBuffered => true,
            Self::ChanFatfs => match std::env::var_os("HADRIS_TESTS_CHAN_FATFS") {
                Some(path) => {
                    assert!(
                        Path::new(&path).is_file(),
                        "configured ChaN helper does not exist"
                    );
                    true
                }
                None => false,
            },
            Self::Mtools => ["mkfs.fat", "mcopy", "mdir"]
                .iter()
                .all(|program| require_or_skip(program, "--help")),
            Self::Xorriso => xorriso::require(),
            Self::Mkisofs(program) => require_or_skip(program, "-version"),
            Self::Bsdtar => require_or_skip("bsdtar", "--version"),
        }
    }
}

#[derive(Default)]
struct Outcome {
    names: Option<Vec<String>>,
    io: Option<IoCounts>,
}

fn created(path: &Path, size: u64) -> File {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.set_len(size).unwrap();
    file
}
fn sync_image(path: &Path) {
    OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .sync_data()
        .unwrap();
}

fn kind(case: FatCase) -> FatKind {
    match case.bits {
        12 => FatKind::Fat12,
        16 => FatKind::Fat16,
        32 => FatKind::Fat32,
        _ => unreachable!(),
    }
}
fn fat_options(case: FatCase) -> FatOptions {
    FatOptions::new()
        .with_kind(kind(case))
        .with_size(case.size)
        .with_label(VolumeLabel::new(LABEL).unwrap())
}

fn hadris_read<F: FileSystem>(mut fs: F, workload: &str, destination: &Path) -> Option<Vec<String>>
where
    F::DeviceError: std::fmt::Debug,
{
    let root = fs.root();
    let listed = entries(&mut fs, root).unwrap();
    if workload == "list" {
        return Some(
            listed
                .iter()
                .map(|entry| {
                    let name = std::str::from_utf8(entry.name().as_bytes()).unwrap();
                    format!("/{}", name.strip_suffix(";1").unwrap_or(name))
                })
                .collect(),
        );
    }
    fs::create_dir_all(destination).unwrap();
    for entry in listed {
        let node = fs.lookup(root, entry.name()).unwrap();
        let contents = read_node(&mut fs, node).unwrap();
        fs.forget(node, 1);
        let name = std::str::from_utf8(entry.name().as_bytes()).unwrap();
        fs::write(
            destination.join(name.strip_suffix(";1").unwrap_or(name)),
            contents,
        )
        .unwrap();
    }
    None
}

#[derive(Clone, Copy)]
struct FatWork<'a> {
    case: FatCase,
    source: &'a Path,
    destination: &'a Path,
    mtools: Option<&'a mtools::MtoolsFatAdapter>,
}

fn chan_job(
    workload: &str,
    image: &Path,
    case: FatCase,
    source: &Path,
    destination: &Path,
) -> Outcome {
    let output = std::process::Command::new(std::env::var_os("HADRIS_TESTS_CHAN_FATFS").unwrap())
        .arg(workload)
        .arg(image)
        .arg(case.bits.to_string())
        .arg(case.size.to_string())
        .arg(if workload == "extract-tree" {
            destination
        } else {
            source
        })
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let values: Vec<u64> = stderr
        .lines()
        .find_map(|line| line.strip_prefix("IO,"))
        .unwrap()
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(values.len(), 7);
    Outcome {
        names: (workload == "list").then(|| {
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }),
        io: Some(IoCounts {
            read_calls: values[0],
            read_bytes: values[1],
            write_calls: values[2],
            write_bytes: values[3],
            flush_calls: values[4],
            failures: values[5],
            seek_calls: values[6],
        }),
    }
}

fn fat_job(peer: Peer, workload: &str, image: &Path, work: FatWork<'_>) -> Outcome {
    let FatWork {
        case,
        source,
        destination,
        mtools,
    } = work;
    let creating = workload == "format-empty" || workload == "create-image";
    match peer {
        Peer::ChanFatfs => chan_job(workload, image, case, source, destination),
        Peer::Hadris => {
            if creating {
                let (mut dev, counts) =
                    Counted::new(FileDevice::new(created(image, case.size)).unwrap());
                if workload == "format-empty" {
                    hadris_fat::sync::format(&mut dev, &fat_options(case)).unwrap();
                } else {
                    let (tree, skipped) =
                        hadris_fs::host::read_tree(source, &hadris_fs::host::TreeOptions::new())
                            .unwrap();
                    assert!(skipped.is_empty());
                    hadris_fat::sync::write(&mut dev, &tree, &fat_options(case)).unwrap();
                }
                Outcome {
                    io: Some(counts.get()),
                    ..Outcome::default()
                }
            } else {
                let (dev, counts) = Counted::new(FileDevice::open(image).unwrap());
                let fs = FatFs::mount(dev, MountOptions::new()).unwrap();
                let names = hadris_read(fs, workload, destination);
                Outcome {
                    names,
                    io: Some(counts.get()),
                }
            }
        }
        Peer::RustFatfs | Peer::RustFatfsBuffered => {
            let file = if creating {
                created(image, case.size)
            } else {
                File::open(image).unwrap()
            };
            let (stream, counts) = CountedStream::new(file);
            if peer == Peer::RustFatfsBuffered {
                rust_job(
                    fscommon::BufStream::new(stream),
                    &counts,
                    workload,
                    case,
                    source,
                    destination,
                )
            } else {
                rust_job(stream, &counts, workload, case, source, destination)
            }
        }
        Peer::Mtools => {
            if creating {
                mtools::format(image, case).unwrap();
            }
            if workload == "format-empty" {
                return Outcome::default();
            }
            let adapter = mtools.unwrap();
            match workload {
                "create-image" => {
                    let mut sources: Vec<_> = fs::read_dir(source)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .collect();
                    sources.sort();
                    adapter.copy_in(&sources).unwrap();
                    Outcome::default()
                }
                "list" => Outcome {
                    names: Some(
                        adapter
                            .list_dir("/")
                            .unwrap()
                            .into_iter()
                            .map(|(name, is_dir)| {
                                assert!(!is_dir);
                                name
                            })
                            .collect(),
                    ),
                    ..Outcome::default()
                },
                _ => {
                    adapter.extract_root(destination).unwrap();
                    Outcome::default()
                }
            }
        }
        _ => unreachable!(),
    }
}

fn rust_job<S: std::io::Read + std::io::Write + std::io::Seek>(
    stream: S,
    counts: &Cell<IoCounts>,
    workload: &str,
    case: FatCase,
    source: &Path,
    destination: &Path,
) -> Outcome {
    let mut stream = fatfs::StdIoWrapper::new(stream);
    let creating = workload == "format-empty" || workload == "create-image";
    if creating {
        let fat_type = match case.bits {
            12 => fatfs::FatType::Fat12,
            16 => fatfs::FatType::Fat16,
            32 => fatfs::FatType::Fat32,
            _ => unreachable!(),
        };
        fatfs::format_volume(
            &mut stream,
            fatfs::FormatVolumeOptions::new()
                .fat_type(fat_type)
                .volume_label(*b"HADRISCONF "),
        )
        .unwrap();
        if workload == "format-empty" {
            fatfs::Write::flush(&mut stream).unwrap();
            return Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            };
        }
    }
    let fs = fatfs::FileSystem::new(stream, fatfs::FsOptions::new()).unwrap();
    let root = fs.root_dir();
    let names = match workload {
        "create-image" => {
            let mut paths: Vec<_> = fs::read_dir(source)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            paths.sort();
            for path in paths {
                let data = fs::read(&path).unwrap();
                root.create_file(path.file_name().unwrap().to_str().unwrap())
                    .unwrap()
                    .write_all(&data)
                    .unwrap();
            }
            None
        }
        "list" => Some(
            root.iter()
                .map(|entry| format!("/{}", entry.unwrap().file_name()))
                .collect(),
        ),
        _ => {
            fs::create_dir_all(destination).unwrap();
            for entry in root.iter() {
                let entry = entry.unwrap();
                let mut bytes = Vec::new();
                entry.to_file().read_to_end(&mut bytes).unwrap();
                fs::write(destination.join(entry.file_name()), bytes).unwrap();
            }
            None
        }
    };
    drop(root);
    fs.unmount().unwrap();
    Outcome {
        names,
        io: Some(counts.get()),
    }
}
fn iso_job(peer: Peer, workload: &str, image: &Path, source: &Path, destination: &Path) -> Outcome {
    if peer == Peer::Hadris {
        if workload == "create-image" {
            let (mut dev, counts) = Counted::new(FileDevice::new(created(image, 0)).unwrap());
            let (tree, skipped) =
                hadris_fs::host::read_tree(source, &hadris_fs::host::TreeOptions::new()).unwrap();
            assert!(skipped.is_empty());
            hadris_iso::sync::write(
                &mut dev,
                &tree,
                &IsoOptions::default().with_id(IsoId::Volume, "PERF"),
            )
            .unwrap();
            return Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            };
        }
        let (dev, counts) = Counted::new(FileDevice::open(image).unwrap());
        let fs = IsoFs::mount(dev, MountOptions::new()).unwrap();
        return Outcome {
            names: hadris_read(fs, workload, destination),
            io: Some(counts.get()),
        };
    }
    if workload == "create-image" {
        if peer == Peer::Xorriso {
            xorriso::mkisofs(source, image, "PERF", &["-iso-level", "1"]).unwrap();
        } else {
            let Peer::Mkisofs(program) = peer else {
                unreachable!()
            };
            run_command(
                program,
                vec![
                    "-iso-level".into(),
                    "1".into(),
                    "-V".into(),
                    "PERF".into(),
                    "-o".into(),
                    image.as_os_str().into(),
                    source.as_os_str().into(),
                ],
            )
            .unwrap();
        }
        return Outcome::default();
    }
    if peer == Peer::Xorriso {
        if workload == "extract-tree" {
            xorriso::extract(image, destination).unwrap();
            return Outcome::default();
        }
        let output = run_command(
            "xorriso",
            vec![
                "-indev".into(),
                image.as_os_str().into(),
                "-ls".into(),
                "/".into(),
            ],
        )
        .unwrap();
        let names = String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(|name| format!("/{}", name.trim_matches('\'')))
            .collect();
        return Outcome {
            names: Some(names),
            ..Outcome::default()
        };
    }
    if peer == Peer::Bsdtar {
        if workload == "extract-tree" {
            fs::create_dir_all(destination).unwrap();
            run_command(
                "bsdtar",
                vec![
                    "-xf".into(),
                    image.as_os_str().into(),
                    "-C".into(),
                    destination.as_os_str().into(),
                ],
            )
            .unwrap();
            return Outcome::default();
        }
        let output = run_command("bsdtar", vec!["-tf".into(), image.as_os_str().into()]).unwrap();
        let names = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter(|name| *name != "." && *name != "./")
            .map(|name| format!("/{}", name.trim_start_matches("./")))
            .collect();
        return Outcome {
            names: Some(names),
            ..Outcome::default()
        };
    }
    unreachable!()
}

fn record(
    rows: &mut Vec<String>,
    peer: Peer,
    format: &str,
    workload: &str,
    samples: usize,
    mut job: impl FnMut() -> Outcome,
    mut verify: impl FnMut(&Outcome) -> u64,
) {
    for sample in 0..=samples {
        let start = Instant::now();
        let result = job();
        let elapsed = start.elapsed().as_nanos();
        let image_bytes = verify(&result);
        if sample == 0 {
            continue;
        }
        let counters = result.io.map_or_else(
            || ",,,,,,".to_string(),
            |io| {
                assert_eq!(io.failures, 0);
                format!(
                    "{},{},{},{},{},{},{}",
                    io.read_calls,
                    io.read_bytes,
                    io.write_calls,
                    io.write_bytes,
                    io.flush_calls,
                    io.failures,
                    io.seek_calls
                )
            },
        );
        rows.push(format!(
            "{},{format},{workload},host-workflow,{sample},{elapsed},{image_bytes},{counters}",
            peer.name()
        ));
    }
    eprintln!(
        "completed {} {format} {workload}: {samples} samples",
        peer.name()
    );
}

fn main() {
    if std::env::var_os("HADRIS_TESTS_PEER_WORKER").is_some() {
        worker::run();
        return;
    }
    if std::env::var_os("HADRIS_TESTS_PROFILE_WORKLOAD").is_some() {
        profile::run();
        return;
    }
    let samples = std::env::var("HADRIS_TESTS_PERF_SAMPLES")
        .map_or(21, |text| text.parse::<usize>().unwrap());
    assert!(samples > 0);
    let filter = std::env::var("HADRIS_TESTS_PERF_FILTER").unwrap_or_default();
    let peer_filter = std::env::var("HADRIS_TESTS_PERF_PEERS").unwrap_or_default();
    let selected = |peer: &Peer| {
        peer_filter.is_empty()
            || peer_filter
                .split(',')
                .any(|name| name.trim() == peer.name())
    };
    let fixture = Fixture::new();
    let workspace = Workspace::new("performance", "peers").unwrap();
    let source = workspace.path.join("source");
    write_host_tree(&source, &fixture.entries).unwrap();
    let image = workspace.path.join("output.img");
    let destination = workspace.path.join("extracted");
    let mut rows = vec!["implementation,format,workload,boundary,sample,elapsed_ns,image_bytes,read_calls,read_bytes,write_calls,write_bytes,flush_calls,io_failures,seek_calls".to_string()];
    let fat_peers: Vec<_> = if filter.is_empty()
        || "fat".contains(&filter)
        || FAT_CASES.iter().any(|case| case.name.contains(&filter))
    {
        [
            Peer::Hadris,
            Peer::RustFatfs,
            Peer::RustFatfsBuffered,
            Peer::Mtools,
            Peer::ChanFatfs,
        ]
        .into_iter()
        .filter(selected)
        .filter(|peer| peer.available())
        .collect()
    } else {
        Vec::new()
    };
    for case in FAT_CASES {
        if !case.name.contains(&filter) {
            continue;
        }
        if case.bits != 32 {
            assert!(
                fixture.entries.len() <= 129,
                "use the fat32 or iso filter for large directories"
            );
        }
        let common = workspace.path.join(format!("{}.img", case.name));
        let work = FatWork {
            case,
            source: &source,
            destination: &destination,
            mtools: None,
        };
        fat_job(Peer::Hadris, "create-image", &common, work);
        let actual: std::collections::BTreeMap<_, _> =
            hadris_tests::fat::spec::snapshot(&common, case.bits)
                .unwrap()
                .entries
                .into_iter()
                .map(|(path, entry)| (path, entry.data))
                .collect();
        assert_eq!(actual, fixture.entries);
        let expected = &fixture.entries;
        for &peer in &fat_peers {
            for workload in ["format-empty", "create-image", "list", "extract-tree"] {
                let input = if workload == "format-empty" || workload == "create-image" {
                    &image
                } else {
                    &common
                };
                let adapter = (peer == Peer::Mtools).then(|| {
                    mtools::MtoolsFatAdapter::new(input.to_path_buf(), &workspace.path).unwrap()
                });
                let work = FatWork {
                    mtools: adapter.as_ref(),
                    ..work
                };
                record(
                    &mut rows,
                    peer,
                    case.name,
                    workload,
                    samples,
                    || {
                        let outcome = fat_job(peer, workload, input, work);
                        if workload == "format-empty" || workload == "create-image" {
                            sync_image(input);
                        }
                        outcome
                    },
                    |outcome| {
                        if let Some(names) = &outcome.names {
                            let mut names = names.clone();
                            names.sort();
                            assert_eq!(names, expected.keys().cloned().collect::<Vec<_>>());
                        } else if workload == "extract-tree" {
                            assert_eq!(&snapshot_host(&destination).unwrap(), expected);
                            fs::remove_dir_all(&destination).unwrap();
                        } else {
                            let actual: std::collections::BTreeMap<_, _> =
                                hadris_tests::fat::spec::snapshot(input, case.bits)
                                    .unwrap()
                                    .entries
                                    .into_iter()
                                    .map(|(path, entry)| (path, entry.data))
                                    .collect();
                            if workload == "format-empty" {
                                assert!(actual.is_empty());
                            } else {
                                assert_eq!(&actual, expected);
                            }
                        }
                        let size = fs::metadata(input).unwrap().len();
                        if workload == "create-image" || workload == "format-empty" {
                            fs::remove_file(input).unwrap();
                        }
                        size
                    },
                );
            }
        }
        assert_eq!(
            rustfatfs::snapshot(&common).unwrap().entries.len(),
            expected.len()
        );
    }
    if "iso".contains(&filter) {
        let program = hadris_tests::iso::mkisofs::CANDIDATES
            .into_iter()
            .find(|program| hadris_tests::harness::program_available(program, "-version"))
            .unwrap_or("mkisofs");
        let peers: Vec<_> = [
            Peer::Hadris,
            Peer::Xorriso,
            Peer::Mkisofs(program),
            Peer::Bsdtar,
        ]
        .into_iter()
        .filter(selected)
        .filter(|peer| peer.available())
        .collect();
        let common = workspace.path.join("common.iso");
        iso_job(Peer::Hadris, "create-image", &common, &source, &destination);
        assert_eq!(
            iso_spec::snapshot(&fs::read(&common).unwrap())
                .unwrap()
                .entries,
            fixture.entries
        );
        for peer in peers {
            for workload in ["create-image", "list", "extract-tree"] {
                if (matches!(peer, Peer::Mkisofs(_)) && workload != "create-image")
                    || (peer == Peer::Bsdtar && workload == "create-image")
                {
                    continue;
                }
                let input = if workload == "create-image" {
                    &image
                } else {
                    &common
                };
                record(
                    &mut rows,
                    peer,
                    "iso",
                    workload,
                    samples,
                    || {
                        let outcome = iso_job(peer, workload, input, &source, &destination);
                        if workload == "create-image" {
                            sync_image(input);
                        }
                        outcome
                    },
                    |outcome| {
                        if let Some(names) = &outcome.names {
                            let mut names = names.clone();
                            names.sort();
                            assert_eq!(names, fixture.entries.keys().cloned().collect::<Vec<_>>());
                        } else if workload == "extract-tree" {
                            assert_eq!(snapshot_host(&destination).unwrap(), fixture.entries);
                            fs::remove_dir_all(&destination).unwrap();
                        } else {
                            assert_eq!(
                                iso_spec::snapshot(&fs::read(input).unwrap())
                                    .unwrap()
                                    .entries,
                                fixture.entries
                            );
                        }
                        let size = fs::metadata(input).unwrap().len();
                        if workload == "create-image" || workload == "format-empty" {
                            fs::remove_file(input).unwrap();
                        }
                        size
                    },
                );
            }
        }
    }
    assert!(rows.len() > 1, "filter matched no formats");
    write_report("performance", "peers.csv", &rows.join("\n")).unwrap();
}

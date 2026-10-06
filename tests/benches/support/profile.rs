use super::*;

pub(super) fn run() {
    let workload = std::env::var("HADRIS_TESTS_PROFILE_WORKLOAD").unwrap();
    assert!(
        [
            "create-image",
            "copy-tree",
            "extract-tree",
            "extract-lazy-tree",
            "lookup-all"
        ]
        .contains(&workload.as_str())
    );
    let implementation =
        std::env::var("HADRIS_TESTS_PROFILE_PEER").unwrap_or_else(|_| "hadris".into());
    let peer = match implementation.as_str() {
        "hadris" => Peer::Hadris,
        "rust-fatfs/unbuffered" => Peer::RustFatfs,
        "rust-fatfs/buffered" => Peer::RustFatfsBuffered,
        _ => panic!("unsupported profile peer"),
    };
    assert!(
        !["lookup-all", "extract-lazy-tree"].contains(&workload.as_str()) || peer == Peer::Hadris
    );
    let seconds = std::env::var("HADRIS_TESTS_PROFILE_SECONDS")
        .map_or(10, |text| text.parse::<u64>().unwrap());
    assert!(seconds > 0);
    let samples = std::env::var("HADRIS_TESTS_PROFILE_SAMPLES")
        .ok()
        .map(|text| text.parse::<usize>().unwrap());
    assert!(samples != Some(0));
    let cached = std::env::var_os("HADRIS_TESTS_PROFILE_CACHE").is_some();
    assert!(!cached || (peer == Peer::Hadris && workload != "create-image"));
    assert!(workload != "copy-tree" || peer == Peer::Hadris);
    let blocks = std::env::var("HADRIS_TESTS_PROFILE_BLOCKS")
        .map_or(16, |text| text.parse::<usize>().unwrap());
    let positions = std::env::var("HADRIS_TESTS_PROFILE_POSITIONS")
        .map_or(256, |text| text.parse::<usize>().unwrap());
    let mut fixture = Fixture::new();
    let directories = std::env::var("HADRIS_TESTS_PROFILE_DIRECTORIES")
        .map_or(0, |text| text.parse::<usize>().unwrap());
    assert!(directories <= 1000);
    let long_names = std::env::var_os("HADRIS_TESTS_PROFILE_LONG_NAMES").is_some();
    assert!((directories == 0 && !long_names) || workload == "extract-lazy-tree");
    if directories != 0 || long_names {
        fixture.entries = fixture
            .entries
            .into_iter()
            .enumerate()
            .map(|(i, (path, entry))| {
                let name = if long_names && !path.ends_with(fixture::PAYLOAD) {
                    format!("Résumé long filename {i:07}.txt")
                } else {
                    path.trim_start_matches('/').to_owned()
                };
                let path = if directories == 0 {
                    format!("/{name}")
                } else {
                    format!("/D{:07}/nested/{name}", i % directories)
                };
                (path, entry)
            })
            .collect();
        for i in 0..directories {
            fixture.entries.insert(
                format!("/D{i:07}"),
                hadris_tests::harness::EntryData::Directory,
            );
            fixture.entries.insert(
                format!("/D{i:07}/nested"),
                hadris_tests::harness::EntryData::Directory,
            );
        }
    }
    let directory_hint = std::env::var_os("HADRIS_TESTS_PROFILE_DIRECTORY_HINT").is_some();
    let directory_entries = std::env::var("HADRIS_TESTS_PROFILE_DIRECTORY_ENTRIES")
        .map_or(fixture.entries.len(), |text| text.parse::<usize>().unwrap());
    let cache_options = hadris_fat::CacheOptions::new()
        .with_blocks(blocks)
        .with_chain_positions(positions)
        .with_directory_entries(directory_entries)
        .with_directory_hint(directory_hint);
    let workspace = Workspace::new("performance", "profile").unwrap();
    let source = workspace.path.join("source");
    write_host_tree(&source, &fixture.entries).unwrap();
    let common = workspace.path.join("common.img");
    let image = workspace.path.join("output.img");
    let destination = workspace.path.join("extracted");
    let work = FatWork {
        case: FAT_CASES[2],
        source: &source,
        destination: &destination,
        mtools: None,
    };
    let (tree, skipped) =
        hadris_fs::host::read_tree(&source, &hadris_fs::host::TreeOptions::new()).unwrap();
    assert!(skipped.is_empty());
    if ["extract-tree", "extract-lazy-tree", "lookup-all"].contains(&workload.as_str()) {
        fat_job(Peer::Hadris, "create-image", &common, work);
        let snapshot = hadris_tests::fat::spec::snapshot(&common, 32).unwrap();
        let actual: std::collections::BTreeMap<_, _> = snapshot
            .entries
            .into_iter()
            .map(|(path, entry)| (path, entry.data))
            .collect();
        assert_eq!(actual, fixture.entries);
    }
    eprintln!(
        "PROFILE_READY pid={} peer={implementation} workload={workload} cached={cached} blocks={blocks} positions={positions} directory_entries={directory_entries} directory_hint={directory_hint}",
        std::process::id()
    );
    let start = Instant::now();
    let mut iterations = 0;
    while samples.map_or_else(
        || start.elapsed().as_secs() < seconds,
        |samples| iterations <= samples,
    ) {
        if destination.exists() {
            fs::remove_dir_all(&destination).unwrap();
        }
        let operation = Instant::now();
        let outcome = if workload == "copy-tree" {
            let (mut dev, counts) =
                Counted::new(FileDevice::new(created(&image, work.case.size)).unwrap());
            hadris_fat::sync::format(&mut dev, &fat_options(work.case)).unwrap();
            let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
            if cached {
                fs = fs.with_cache(cache_options);
            }
            let root = fs.root();
            hadris_fs::sync::copy_tree(&tree, &mut fs, root).unwrap();
            fs.unmount().unwrap();
            sync_image(&image);
            Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            }
        } else if workload == "extract-lazy-tree" {
            let counts = std::sync::Arc::new(std::sync::Mutex::new(IoCounts::default()));
            let dev = SendCounted {
                inner: FileDevice::open(&common).unwrap(),
                counts: counts.clone(),
            };
            let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
            if cached {
                fs = fs.with_cache(cache_options);
            }
            let vol = hadris_fs::sync::Volume::new(fs);
            let tree = hadris_fs::sync::read_tree(&vol, "/").unwrap();
            hadris_fs::host::write_tree(&destination, &tree).unwrap();
            let io = *counts.lock().unwrap();
            Outcome {
                io: Some(io),
                ..Outcome::default()
            }
        } else if workload == "lookup-all" || (workload == "extract-tree" && cached) {
            assert!(peer == Peer::Hadris);
            let (dev, counts) = Counted::new(FileDevice::open(&common).unwrap());
            let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
            if cached {
                fs = fs.with_cache(cache_options);
            }
            if workload == "lookup-all" {
                let root = fs.root();
                for path in fixture.entries.keys() {
                    let name = hadris_fs::Name::new(&path.as_bytes()[1..]);
                    let node = fs.lookup(root, name).unwrap();
                    fs.forget(node, 1);
                }
            } else {
                hadris_read(fs, &workload, &destination);
            }
            Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            }
        } else {
            let outcome = fat_job(
                peer,
                &workload,
                if workload == "create-image" {
                    &image
                } else {
                    &common
                },
                work,
            );
            if workload == "create-image" {
                sync_image(&image);
            }
            outcome
        };
        let elapsed = operation.elapsed().as_nanos();
        let io = outcome.io.unwrap();
        assert_eq!(io.failures, 0);
        eprintln!(
            "PROFILE_SAMPLE ns={elapsed} reads={} read_bytes={} writes={} seeks={}",
            io.read_calls, io.read_bytes, io.write_calls, io.seek_calls
        );
        iterations += 1;
    }
    if workload == "extract-tree" || workload == "extract-lazy-tree" {
        assert_eq!(snapshot_host(&destination).unwrap(), fixture.entries);
    }
    if workload == "create-image" || workload == "copy-tree" {
        let actual: std::collections::BTreeMap<_, _> =
            hadris_tests::fat::spec::snapshot(&image, 32)
                .unwrap()
                .entries
                .into_iter()
                .map(|(path, entry)| (path, entry.data))
                .collect();
        assert_eq!(actual, fixture.entries);
    }
    eprintln!("PROFILE_DONE iterations={iterations}");
}

struct SendCounted {
    inner: FileDevice,
    counts: std::sync::Arc<std::sync::Mutex<IoCounts>>,
}

impl hadris_io::ErrorType for SendCounted {
    type Error = std::io::Error;
}

impl hadris_storage::sync::BlockDevice for SendCounted {
    fn block_size(&self) -> hadris_storage::BlockSize {
        self.inner.block_size()
    }
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn disk_offset(&self) -> u64 {
        self.inner.disk_offset()
    }
    fn writable(&self) -> bool {
        self.inner.writable()
    }
    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        let result = self.inner.read_blocks(first, buf);
        let mut counts = self.counts.lock().unwrap();
        counts.read_calls += 1;
        counts.read_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        result
    }
    fn write_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        let result = self.inner.write_blocks(first, buf);
        let mut counts = self.counts.lock().unwrap();
        counts.write_calls += 1;
        counts.write_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        result
    }
    fn flush(&mut self) -> Result<(), hadris_io::Error<Self::Error>> {
        let result = self.inner.flush();
        let mut counts = self.counts.lock().unwrap();
        counts.flush_calls += 1;
        counts.failures += u64::from(result.is_err());
        result
    }
}

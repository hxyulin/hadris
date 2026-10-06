use super::*;

pub(super) fn run() {
    let peer = std::env::var("HADRIS_TESTS_PEER_WORKER").unwrap();
    let image = std::path::PathBuf::from(std::env::var_os("HADRIS_TESTS_PEER_IMAGE").unwrap());
    let destination = std::path::PathBuf::from(
        std::env::var_os("HADRIS_TESTS_PEER_DESTINATION").unwrap_or_default(),
    );
    let case = FAT_CASES[2];
    let work = FatWork {
        case,
        source: Path::new("."),
        destination: &destination,
        mtools: None,
    };
    if peer == "prepare" {
        let fixture = Fixture::new();
        let workspace = Workspace::new("performance", "worker-prepare").unwrap();
        let source = workspace.path.join("source");
        write_host_tree(&source, &fixture.entries).unwrap();
        fat_job(
            Peer::Hadris,
            "create-image",
            &image,
            FatWork {
                source: &source,
                ..work
            },
        );
        let actual: std::collections::BTreeMap<_, _> =
            hadris_tests::fat::spec::snapshot(&image, 32)
                .unwrap()
                .entries
                .into_iter()
                .map(|(path, entry)| (path, entry.data))
                .collect();
        assert_eq!(actual, fixture.entries);
        eprintln!("WORKER_PREPARED entries={}", actual.len());
        return;
    }
    assert!(
        !destination.exists(),
        "extraction destination must be fresh"
    );
    let cache = std::env::var("HADRIS_TESTS_PEER_CACHE").unwrap_or_else(|_| "none".into());
    let configured = match cache.as_str() {
        "none" => None,
        "hint" => Some(hadris_fat::CacheOptions::sequential()),
        "index" => Some(
            hadris_fat::CacheOptions::new()
                .with_blocks(0)
                .with_chain_positions(0)
                .with_directory_entries(
                    std::env::var("HADRIS_TESTS_PEER_DIRECTORY_ENTRIES")
                        .map_or(1001, |text| text.parse::<usize>().unwrap()),
                ),
        ),
        _ => panic!("unsupported worker cache"),
    };
    let start = Instant::now();
    let outcome = match peer.as_str() {
        "hadris/lazy" => profile::lazy_extract(&image, &destination, configured),
        "hadris/stream" => {
            let (dev, counts) = Counted::new(FileDevice::open(&image).unwrap());
            let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
            if let Some(cache) = configured {
                fs = fs.with_cache(cache);
            }
            let root = fs.root();
            let mut cursor = hadris_fs::DirCursor::START;
            fs::create_dir_all(&destination).unwrap();
            while let Some(entry) = fs.readdir(root, cursor).unwrap() {
                cursor = entry.next_cursor();
                assert!(entry.metadata().file_type().is_file());
                let node = fs.lookup(root, entry.name()).unwrap();
                let contents = read_node(&mut fs, node).unwrap();
                fs.forget(node, 1);
                let name = entry.name().to_str().unwrap();
                fs::write(destination.join(name), contents).unwrap();
            }
            Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            }
        }
        "rust-fatfs/unbuffered" | "rust-fatfs/buffered" => {
            assert!(configured.is_none());
            fat_job(
                if peer.ends_with("/unbuffered") {
                    Peer::RustFatfs
                } else {
                    Peer::RustFatfsBuffered
                },
                "extract-tree",
                &image,
                work,
            )
        }
        _ => panic!("unsupported worker peer"),
    };
    let elapsed = start.elapsed().as_nanos();
    let io = outcome.io.unwrap();
    assert_eq!(io.failures, 0);
    let driver_bytes = if peer.starts_with("hadris/") {
        std::mem::size_of::<FatFs<FileDevice>>()
    } else {
        0
    };
    eprintln!(
        "WORKER_SAMPLE ns={elapsed} reads={} read_bytes={} writes={} failures={} driver_bytes={}",
        io.read_calls, io.read_bytes, io.write_calls, io.failures, driver_bytes
    );
}

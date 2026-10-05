use super::*;

pub(super) fn run() {
    let workload = std::env::var("HADRIS_TESTS_PROFILE_WORKLOAD").unwrap();
    assert!(
        ["create-image", "copy-tree", "extract-tree", "lookup-all"].contains(&workload.as_str())
    );
    let implementation =
        std::env::var("HADRIS_TESTS_PROFILE_PEER").unwrap_or_else(|_| "hadris".into());
    let peer = match implementation.as_str() {
        "hadris" => Peer::Hadris,
        "rust-fatfs/unbuffered" => Peer::RustFatfs,
        "rust-fatfs/buffered" => Peer::RustFatfsBuffered,
        _ => panic!("unsupported profile peer"),
    };
    assert!(workload != "lookup-all" || peer == Peer::Hadris);
    let seconds = std::env::var("HADRIS_TESTS_PROFILE_SECONDS")
        .map_or(10, |text| text.parse::<u64>().unwrap());
    assert!(seconds > 0);
    let cached = std::env::var_os("HADRIS_TESTS_PROFILE_CACHE").is_some();
    assert!(!cached || (peer == Peer::Hadris && workload != "create-image"));
    assert!(workload != "copy-tree" || peer == Peer::Hadris);
    let blocks = std::env::var("HADRIS_TESTS_PROFILE_BLOCKS")
        .map_or(16, |text| text.parse::<usize>().unwrap());
    let fixture = Fixture::new();
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
    if workload == "extract-tree" || workload == "lookup-all" {
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
        "PROFILE_READY pid={} peer={implementation} workload={workload} cached={cached}",
        std::process::id()
    );
    let start = Instant::now();
    let mut iterations = 0;
    while start.elapsed().as_secs() < seconds {
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
                fs = fs.with_cache(
                    hadris_fat::CacheOptions::new()
                        .with_blocks(blocks)
                        .with_chain_positions(256)
                        .with_directory_entries(fixture.entries.len()),
                );
            }
            let root = fs.root();
            hadris_fs::sync::copy_tree(&tree, &mut fs, root).unwrap();
            fs.unmount().unwrap();
            sync_image(&image);
            Outcome {
                io: Some(counts.get()),
                ..Outcome::default()
            }
        } else if workload == "lookup-all" || (workload == "extract-tree" && cached) {
            assert!(peer == Peer::Hadris);
            let (dev, counts) = Counted::new(FileDevice::open(&common).unwrap());
            let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
            if cached {
                fs = fs.with_cache(
                    hadris_fat::CacheOptions::new()
                        .with_blocks(blocks)
                        .with_chain_positions(256)
                        .with_directory_entries(fixture.entries.len()),
                );
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
    if workload == "extract-tree" {
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

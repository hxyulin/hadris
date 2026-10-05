#[path = "common/fatfs.rs"]
mod common;
#[allow(dead_code)]
#[path = "../benches/support/mod.rs"]
mod support;

use common::{CASES, FsPaths};
use hadris_fat::CacheOptions;
use hadris_fat::sync::FatFs;
use hadris_fs::sync::FileSystem;
use hadris_fs::{ErrorKind, MountOptions, Name, RenameMode, SetAttr};
use std::cell::Cell;
use support::{Counted, IoCounts};

fn fixture(case: common::Case, bytes: usize) -> (Vec<u8>, Vec<u8>) {
    let data: Vec<_> = (0..bytes).map(|i| ((i * 31) ^ (i >> 9)) as u8).collect();
    let mut fs = FatFs::mount(
        common::device(case, common::blank(case)),
        MountOptions::new(),
    )
    .unwrap();
    let file = fs
        .create(fs.root(), Name::new("DATA.BIN"), &SetAttr::new())
        .unwrap();
    assert_eq!(fs.write(file, 0, &data).unwrap(), bytes);
    (fs.unmount().unwrap().into_inner(), data)
}

#[test]
fn chain_index_reduces_reverse_reads_without_changing_contents() {
    for case in CASES[..3].iter().copied() {
        let (image, data) = fixture(case, 1 << 20);
        let reads = |capacity| {
            let counts = Cell::new(IoCounts::default());
            let dev = Counted {
                inner: common::device(case, image.clone()),
                counts: &counts,
                written_blocks: None,
            };
            let mut fs = FatFs::mount(dev, MountOptions::new().read_only())
                .unwrap()
                .with_cache(
                    CacheOptions::new()
                        .with_chain_positions(capacity)
                        .with_blocks(0),
                );
            let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
            counts.set(IoCounts::default());
            for offset in (0..data.len()).step_by(4096).rev() {
                let mut buf = [0; 4096];
                assert_eq!(fs.read(file, offset as u64, &mut buf).unwrap(), buf.len());
                assert_eq!(&buf, &data[offset..offset + buf.len()]);
            }
            assert_eq!(counts.get().write_calls, 0);
            counts.get().read_calls
        };
        let uncached = reads(0);
        let cached = reads(32);
        assert!(
            cached < uncached / 2,
            "{}: {cached} vs {uncached}",
            case.name
        );
    }
}

#[test]
fn indexed_small_forward_reads_and_later_seeks_match_uncached_reads() {
    for case in CASES {
        let (image, data) = fixture(case, 32768);
        for blocks in [0, 8] {
            let mut fs = FatFs::mount(
                common::device(case, image.clone()),
                MountOptions::new().read_only(),
            )
            .unwrap()
            .with_cache(
                CacheOptions::new()
                    .with_chain_positions(4)
                    .with_blocks(blocks),
            );
            let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
            let mut offset = 0;
            let mut buf = [0; 73];
            while offset < data.len() {
                let n = fs.read(file, offset as u64, &mut buf).unwrap();
                assert_eq!(&buf[..n], &data[offset..offset + n]);
                offset += n;
            }
            for offset in [1, 16383, 4093, 32000, 512, 0] {
                let n = fs.read(file, offset as u64, &mut buf).unwrap();
                assert_eq!(&buf[..n], &data[offset..offset + n]);
            }
            assert_eq!(fs.read(file, data.len() as u64, &mut buf).unwrap(), 0);
            fs.clear_cache();
            assert_eq!(fs.read(file, 1, &mut buf).unwrap(), buf.len());
            assert_eq!(&buf, &data[1..1 + buf.len()]);
        }
    }
}

#[test]
fn chain_index_reads_fragmented_files_and_resets_after_mutations() {
    for case in CASES {
        let mut fs = FatFs::mount(
            common::device(case, common::build(case)),
            MountOptions::new(),
        )
        .unwrap()
        .with_cache(CacheOptions::new().with_chain_positions(8));
        let file = fs.resolve_path("/frag.bin").unwrap();
        let mut expected = common::payload(100, 2);
        expected.extend(common::payload(40_000, 4));
        for offset in [39000, 1, 32768, 4093, 20000, 0] {
            let mut buf = [0; 1093];
            let n = fs.read(file, offset, &mut buf).unwrap();
            assert_eq!(&buf[..n], &expected[offset as usize..offset as usize + n]);
        }
        fs.truncate(file, 600).unwrap();
        fs.write(file, 40000, b"tail").unwrap();
        expected.truncate(600);
        expected.resize(40000, 0);
        expected.extend_from_slice(b"tail");
        for offset in [40000, 32768, 1, 4093] {
            let mut buf = [0; 4096];
            let n = fs.read(file, offset, &mut buf).unwrap();
            assert_eq!(&buf[..n], &expected[offset as usize..offset as usize + n]);
        }
        let root = fs.root();
        fs.rename(
            root,
            Name::new("frag.bin"),
            root,
            Name::new("moved.bin"),
            RenameMode::NoReplace,
        )
        .unwrap();
        assert_eq!(fs.read(file, 40000, &mut [0; 4]).unwrap(), 4);
        fs.unlink(root, Name::new("moved.bin")).unwrap();
        assert_eq!(
            fs.read(file, 0, &mut [0; 4]).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        fs.forget(file, 1);
        let replacement = fs
            .create(root, Name::new("moved.bin"), &SetAttr::new())
            .unwrap();
        fs.write(replacement, 0, b"replacement").unwrap();
        fs.clear_cache();
        let mut buf = [0; 11];
        assert_eq!(fs.read(replacement, 0, &mut buf).unwrap(), 11);
        assert_eq!(&buf, b"replacement");
        let image = fs.unmount().unwrap().into_inner();
        common::assert_checks_clean(case, &image, case.name);
    }
}

#[test]
fn cached_positions_preserve_corrupt_chain_detection() {
    for case in CASES[..3].iter().copied() {
        let (mut image, _) = fixture(case, 32768);
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        let chain = {
            let mut fs =
                FatFs::mount(common::device(case, image.clone()), MountOptions::new()).unwrap();
            let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
            common::chain(&mut fs, file)
        };
        for copy in 0..geo.fat_count() {
            let at = (geo.fat_copy(copy) + case.kind.entry_offset(chain[8] as u64)) as usize;
            case.kind.encode(
                chain[8] as u64,
                chain[4],
                &mut image[at..at + case.kind.entry_len()],
            );
        }
        let mut fs = FatFs::mount(common::device(case, image), MountOptions::new().read_only())
            .unwrap()
            .with_cache(CacheOptions::new().with_chain_positions(4));
        let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
        fs.read(file, 0, &mut [0; 512]).unwrap();
        assert_eq!(
            fs.read(file, 30000, &mut [0; 512]).unwrap_err().kind(),
            ErrorKind::Corrupt
        );
        assert_eq!(
            fs.read(file, 0, &mut [0; 32768]).unwrap_err().kind(),
            ErrorKind::Corrupt
        );
    }
}

#[cfg(feature = "async")]
#[path = "common/cancel.rs"]
mod cancel;

#[cfg(feature = "async")]
#[test]
fn cancelled_index_walks_resume_correctly() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    let case = CASES[2];
    let (image, data) = fixture(case, 32768);
    for budget in 0..40 {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new().read_only(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(CacheOptions::new().with_chain_positions(8));
        let file = cancel::run_for(fs.lookup(fs.root(), Name::new("DATA.BIN")), usize::MAX)
            .unwrap()
            .unwrap();
        let mut buf = [0; 4096];
        let result = cancel::run_for(fs.read(file, 28672, &mut buf), budget);
        let n = cancel::run_for(fs.read(file, 8192, &mut buf), usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(n, buf.len());
        assert_eq!(&buf, &data[8192..12288]);
        fs.clear_cache();
        cancel::run_for(fs.read(file, 28672, &mut buf), usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(&buf, &data[28672..32768]);
        if result.is_some() {
            return;
        }
    }
    panic!("indexed read never completed");
}

#[cfg(feature = "async")]
#[test]
fn cancelled_growth_discards_cached_positions() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    let case = CASES[2];
    let (image, data) = fixture(case, 32768);
    for budget in 0..80 {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(CacheOptions::new().with_chain_positions(8));
        let file = cancel::run_for(fs.lookup(fs.root(), Name::new("DATA.BIN")), usize::MAX)
            .unwrap()
            .unwrap();
        let mut buf = [0; 4096];
        cancel::run_for(fs.read(file, 28672, &mut buf), usize::MAX)
            .unwrap()
            .unwrap();
        let result = cancel::run_for(fs.write(file, data.len() as u64, &[9]), budget);
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        cancel::run_for(fs.read(file, 8192, &mut buf), usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(&buf, &data[8192..12288]);
        let len = cancel::run_for(fs.stat(file), usize::MAX)
            .unwrap()
            .unwrap()
            .len();
        assert_eq!(len, data.len() as u64 + u64::from(result.is_some()));
        let image = cancel::run_for(fs.unmount(), usize::MAX)
            .unwrap()
            .unwrap()
            .0
            .into_inner();
        common::assert_checks_clean(case, &image, "cancelled cached growth");
        if result.is_some() {
            return;
        }
    }
    panic!("cached growth never completed");
}

#[test]
fn metadata_cache_survives_payload_reads_and_never_caches_aligned_payload() {
    let case = CASES[2];
    let (image, data) = fixture(case, 32768);
    let counts = Cell::new(IoCounts::default());
    let dev = Counted {
        inner: common::device(case, image),
        counts: &counts,
        written_blocks: None,
    };
    let mut fs = FatFs::mount(dev, MountOptions::new().read_only())
        .unwrap()
        .with_cache(CacheOptions::new().with_chain_positions(0).with_blocks(8));
    let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
    fs.read(file, 1, &mut [0; 1]).unwrap();
    counts.set(IoCounts::default());
    let again = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
    assert_eq!(again, file);
    assert_eq!(counts.get().read_calls, 0);
    for _ in 0..2 {
        let mut buf = [0; 512];
        fs.read(file, 0, &mut buf).unwrap();
        assert_eq!(&buf, &data[..512]);
    }
    assert_eq!(counts.get().read_calls, 2);
    fs.clear_cache();
    fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
    assert_eq!(counts.get().read_calls, 3);
}

#[test]
fn metadata_cache_recovers_after_a_failed_mirror_write() {
    use common::script::Scripted;
    for case in CASES[..3].iter().copied() {
        let (image, data) = fixture(case, 32768);
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        let (dev, script) = Scripted::new(common::device(case, image));
        let mut fs = FatFs::mount(dev, MountOptions::new())
            .unwrap()
            .with_cache(CacheOptions::new());
        let file = fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap();
        let mut buf = [0; 4096];
        fs.read(file, 28672, &mut buf).unwrap();
        script.borrow_mut().fail_at = Some(
            geo.fat_copy(1)
                + case
                    .kind
                    .entry_offset(common::chain(&mut fs, file)[0] as u64),
        );
        assert_eq!(
            fs.write(file, data.len() as u64, &[9]).unwrap_err().kind(),
            ErrorKind::Io
        );
        fs.sync().unwrap();
        assert_eq!(fs.stat(file).unwrap().len(), data.len() as u64);
        fs.read(file, 8192, &mut buf).unwrap();
        assert_eq!(&buf, &data[8192..12288]);
        let image = fs.unmount().unwrap().into_image();
        common::assert_checks_clean(case, &image, "failed cached mirror");
    }
}

fn directory_fixture(case: common::Case) -> (Vec<u8>, Vec<String>) {
    let mut fs = FatFs::mount(
        common::device(case, common::blank(case)),
        MountOptions::new(),
    )
    .unwrap();
    let names: Vec<_> = (0..48)
        .map(|i| format!("Résumé supplementary 😀 long filename {i:03}.txt"))
        .collect();
    for (i, name) in names.iter().enumerate() {
        let file = fs
            .create(fs.root(), Name::new(name), &SetAttr::new())
            .unwrap();
        fs.write(file, 0, &[i as u8]).unwrap();
        fs.close(file).unwrap();
    }
    (fs.unmount().unwrap().into_inner(), names)
}

#[test]
fn directory_index_learns_prefix_and_bounds_overflow_without_changing_names() {
    for case in CASES {
        let (image, names) = directory_fixture(case);
        let run = |capacity| {
            let counts = Cell::new(IoCounts::default());
            let dev = Counted {
                inner: common::device(case, image.clone()),
                counts: &counts,
                written_blocks: None,
            };
            let mut fs = FatFs::mount(dev, MountOptions::new().read_only())
                .unwrap()
                .with_cache(
                    CacheOptions::new()
                        .with_chain_positions(0)
                        .with_blocks(0)
                        .with_directory_entries(capacity),
                );
            counts.set(IoCounts::default());
            for (i, name) in names.iter().enumerate() {
                let file = fs
                    .lookup(fs.root(), Name::new(&name.to_lowercase()))
                    .unwrap();
                let mut byte = [0];
                fs.read(file, 0, &mut byte).unwrap();
                assert_eq!(byte, [i as u8]);
                fs.close(file).unwrap();
            }
            let first_pass = counts.get().read_calls;
            counts.set(IoCounts::default());
            for name in names.iter().rev() {
                let file = fs.lookup(fs.root(), Name::new(name)).unwrap();
                fs.close(file).unwrap();
            }
            let repeated = counts.get().read_calls;
            assert_eq!(
                fs.lookup(fs.root(), Name::new("missing long name"))
                    .unwrap_err()
                    .kind(),
                ErrorKind::NotFound
            );
            fs.clear_cache();
            fs.lookup(fs.root(), Name::new(&names[0])).unwrap();
            (first_pass, repeated)
        };
        let baseline = run(0);
        let indexed = run(48);
        assert!(
            indexed.0 < baseline.0,
            "{}: {indexed:?} {baseline:?}",
            case.name
        );
        assert_eq!(indexed.1, 0, "{}", case.name);
        for capacity in [1, 4] {
            run(capacity);
        }
    }
}

#[test]
fn directory_index_invalidates_mutations_and_preserves_dirty_pins() {
    for case in CASES {
        let mut fs = FatFs::mount(
            common::device(case, common::blank(case)),
            MountOptions::new(),
        )
        .unwrap()
        .with_cache(CacheOptions::new().with_directory_entries(8));
        let root = fs.root();
        let a = fs
            .create(root, Name::new("OLD.TXT"), &SetAttr::new())
            .unwrap();
        let dir = fs.mkdir(root, Name::new("DIR"), &SetAttr::new()).unwrap();
        fs.lookup(root, Name::new("OLD.TXT")).unwrap();
        fs.write(a, 0, &[7; 20]).unwrap();
        assert_eq!(fs.lookup(root, Name::new("old.txt")).unwrap(), a);
        fs.sync().unwrap();
        fs.forget(a, 3);
        let a = fs.lookup(root, Name::new("OLD.TXT")).unwrap();
        assert_eq!(fs.stat(a).unwrap().len(), 20);
        fs.rename(
            root,
            Name::new("OLD.TXT"),
            dir,
            Name::new("Moved long name.txt"),
            RenameMode::NoReplace,
        )
        .unwrap();
        assert_eq!(
            fs.lookup(root, Name::new("OLD.TXT")).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        let moved = fs.lookup(dir, Name::new("moved long name.txt")).unwrap();
        fs.unlink(dir, Name::new("Moved long name.txt")).unwrap();
        assert_eq!(
            fs.lookup(dir, Name::new("Moved long name.txt"))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        let replacement = fs
            .create(dir, Name::new("REPLACED.TXT"), &SetAttr::new())
            .unwrap();
        fs.write(replacement, 0, &[9]).unwrap();
        let replacement = fs.lookup(dir, Name::new("replaced.txt")).unwrap();
        let mut byte = [0];
        fs.read(replacement, 0, &mut byte).unwrap();
        assert_eq!(byte, [9]);
        fs.forget(moved, 1);
        fs.sync().unwrap();
        let image = fs.unmount().unwrap().into_inner();
        common::assert_checks_clean(case, &image, "indexed directory mutation");
    }
}

#[cfg(feature = "async")]
#[test]
fn directory_prefix_survives_lookup_cancellation_at_every_await() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    let case = CASES[2];
    let (image, names) = directory_fixture(case);
    for budget in 0..200 {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new().read_only(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(
            CacheOptions::new()
                .with_blocks(0)
                .with_directory_entries(48),
        );
        let result = cancel::run_for(fs.lookup(fs.root(), Name::new(&names[47])), budget);
        for name in &names {
            let file = cancel::run_for(fs.lookup(fs.root(), Name::new(name)), usize::MAX)
                .unwrap()
                .unwrap();
            cancel::run_for(fs.close(file), usize::MAX)
                .unwrap()
                .unwrap();
        }
        if result.is_some() {
            return;
        }
    }
    panic!("indexed lookup never completed");
}

#[test]
fn directory_index_matches_long_names_and_their_short_aliases() {
    let case = CASES[0];
    let (image, names) = directory_fixture(case);
    let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
    let hadris_fat_raw::RootLocation::Fixed { start, size } = geo.root() else {
        unreachable!()
    };
    let aliases: Vec<_> = image[start as usize..(start + size) as usize]
        .chunks_exact(32)
        .filter_map(|bytes| {
            let hadris_fat_raw::Slot::Short(entry) =
                hadris_fat_raw::Slot::parse(bytes.try_into().unwrap())
            else {
                return None;
            };
            if !entry.is_visible() {
                return None;
            }
            let mut buf = [0; hadris_fat_raw::short_name::DISPLAY_MAX];
            let len = hadris_fat_raw::short_name::display(
                &entry.name(),
                entry.nt_case(),
                |b| hadris_fs::CodePage::decode(&hadris_fs::Cp437, b),
                &mut buf,
            );
            Some(String::from_utf8(buf[..len].to_vec()).unwrap())
        })
        .collect();
    assert_eq!(aliases.len(), names.len());
    let mut fs = FatFs::mount(common::device(case, image), MountOptions::new().read_only())
        .unwrap()
        .with_cache(CacheOptions::new().with_directory_entries(48));
    for (name, alias) in names.iter().zip(&aliases) {
        let long = fs.lookup(fs.root(), Name::new(name)).unwrap();
        assert_eq!(
            fs.lookup(fs.root(), Name::new(&alias.to_lowercase()))
                .unwrap(),
            long
        );
        fs.forget(long, 2);
    }
    for (name, alias) in names.iter().zip(&aliases).rev() {
        let short = fs.lookup(fs.root(), Name::new(alias)).unwrap();
        assert_eq!(fs.lookup(fs.root(), Name::new(name)).unwrap(), short);
        fs.forget(short, 2);
    }
}

#[test]
fn directory_prefix_keeps_chain_guard_when_missing_names_encounter_cycles() {
    let case = CASES[2];
    let (mut image, names) = directory_fixture(case);
    let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
    let hadris_fat_raw::RootLocation::Cluster(first) = geo.root() else {
        unreachable!()
    };
    let mut cluster = first;
    loop {
        let base = geo.cluster_offset(cluster).unwrap() as usize;
        for slot in image[base..base + geo.cluster_size() as usize].chunks_exact_mut(32) {
            if slot[0] == 0 {
                slot[0] = 0xe5;
            }
        }
        let at = (geo.fat_copy(0) + case.kind.entry_offset(cluster as u64)) as usize;
        let next = case
            .kind
            .decode(cluster as u64, &image[at..at + case.kind.entry_len()]);
        if case.kind.is_end_of_chain(next) {
            for copy in 0..geo.fat_count() {
                let at = (geo.fat_copy(copy) + case.kind.entry_offset(cluster as u64)) as usize;
                case.kind.encode(
                    cluster as u64,
                    first,
                    &mut image[at..at + case.kind.entry_len()],
                );
            }
            break;
        }
        cluster = next;
    }
    for capacity in [1, 48, 128] {
        let mut fs = FatFs::mount(
            common::device(case, image.clone()),
            MountOptions::new().read_only(),
        )
        .unwrap()
        .with_cache(CacheOptions::new().with_directory_entries(capacity));
        fs.lookup(fs.root(), Name::new(&names[0])).unwrap();
        for _ in 0..2 {
            assert_eq!(
                fs.lookup(fs.root(), Name::new("missing long name"))
                    .unwrap_err()
                    .kind(),
                ErrorKind::Corrupt
            );
        }
        fs.lookup(fs.root(), Name::new(&names[0])).unwrap();
    }
}

#[cfg(feature = "async")]
#[test]
fn directory_index_discards_names_before_cancelled_rename_writes() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    let case = CASES[2];
    let mut original = hadris_fat::sync::FatFs::mount(
        common::device(case, common::blank(case)),
        MountOptions::new(),
    )
    .unwrap();
    let file = original
        .create(original.root(), Name::new("OLD.TXT"), &SetAttr::new())
        .unwrap();
    original.close(file).unwrap();
    let image = original.unmount().unwrap().into_inner();
    for budget in 0..100 {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(CacheOptions::new().with_directory_entries(8));
        let root = fs.root();
        cancel::run_for(fs.lookup(root, Name::new("OLD.TXT")), usize::MAX)
            .unwrap()
            .unwrap();
        let result = cancel::run_for(
            fs.rename(
                root,
                Name::new("OLD.TXT"),
                root,
                Name::new("A replacement long name.txt"),
                RenameMode::NoReplace,
            ),
            budget,
        );
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        for name in ["OLD.TXT", "A replacement long name.txt"] {
            let before = cancel::run_for(fs.lookup(root, Name::new(name)), usize::MAX)
                .unwrap()
                .map_err(|err| err.kind());
            fs.clear_cache();
            let after = cancel::run_for(fs.lookup(root, Name::new(name)), usize::MAX)
                .unwrap()
                .map_err(|err| err.kind());
            assert_eq!(before, after);
            if let Ok(file) = after {
                assert_eq!(
                    cancel::run_for(fs.stat(file), usize::MAX)
                        .unwrap()
                        .unwrap()
                        .len(),
                    0
                );
            }
        }
        let image = cancel::run_for(fs.unmount(), usize::MAX)
            .unwrap()
            .unwrap()
            .0
            .into_inner();
        common::assert_checks_clean(
            case,
            &image,
            &format!("cancelled indexed rename budget {budget}"),
        );
        if result.is_some() {
            return;
        }
    }
    panic!("indexed rename never completed");
}

#[cfg(feature = "async")]
#[test]
#[ignore = "pre-existing cancelled rename can leave cross-linked entries; directory indexing is disabled"]
fn cancelled_nonempty_rename_can_leave_a_cross_link_without_directory_indexing() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    let case = CASES[2];
    let mut original = hadris_fat::sync::FatFs::mount(
        common::device(case, common::blank(case)),
        MountOptions::new(),
    )
    .unwrap();
    let file = original
        .create(original.root(), Name::new("OLD.TXT"), &SetAttr::new())
        .unwrap();
    original.write(file, 0, b"preserved").unwrap();
    let image = original.unmount().unwrap().into_inner();
    let mut fs = cancel::run_for(
        FatFs::mount(
            cancel::YieldDev(common::device(case, image)),
            MountOptions::new(),
        ),
        usize::MAX,
    )
    .unwrap()
    .unwrap()
    .with_cache(CacheOptions::new().with_directory_entries(0));
    let root = fs.root();
    cancel::run_for(fs.lookup(root, Name::new("OLD.TXT")), usize::MAX)
        .unwrap()
        .unwrap();
    assert!(
        cancel::run_for(
            fs.rename(
                root,
                Name::new("OLD.TXT"),
                root,
                Name::new("A replacement long name.txt"),
                RenameMode::NoReplace
            ),
            5
        )
        .is_none()
    );
    cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
    let image = cancel::run_for(fs.unmount(), usize::MAX)
        .unwrap()
        .unwrap()
        .0
        .into_inner();
    common::assert_checks_clean(
        case,
        &image,
        "cancelled nonempty rename without directory indexing",
    );
}

#[test]
fn append_planning_preserves_collisions_holes_and_capacity_fallback() {
    for case in CASES {
        for capacity in [1, 8, 64] {
            let mut fs = FatFs::mount(
                common::device(case, common::blank(case)),
                MountOptions::new(),
            )
            .unwrap()
            .with_cache(
                CacheOptions::new()
                    .with_blocks(0)
                    .with_directory_entries(capacity),
            );
            let root = fs.root();
            for i in 0..40 {
                let name = format!("F{i:07}.TXT");
                let file = fs.create(root, Name::new(&name), &SetAttr::new()).unwrap();
                fs.write(file, 0, &[i as u8]).unwrap();
                fs.close(file).unwrap();
                fs.forget(file, 1);
                assert_eq!(
                    fs.create(root, Name::new(&name.to_ascii_lowercase()), &SetAttr::new())
                        .unwrap_err()
                        .kind(),
                    ErrorKind::AlreadyExists
                );
            }
            fs.unlink(root, Name::new("F0000003.TXT")).unwrap();
            let replacement = fs
                .create(root, Name::new("REUSED.TXT"), &SetAttr::new())
                .unwrap();
            fs.write(replacement, 0, b"new").unwrap();
            fs.forget(replacement, 1);
            fs.rename(
                root,
                Name::new("F0000004.TXT"),
                root,
                Name::new("RENAMED.TXT"),
                RenameMode::NoReplace,
            )
            .unwrap();
            assert_eq!(
                fs.create(root, Name::new("renamed.txt"), &SetAttr::new())
                    .unwrap_err()
                    .kind(),
                ErrorKind::AlreadyExists
            );
            let long = fs
                .create(root, Name::new("A long file name.txt"), &SetAttr::new())
                .unwrap();
            fs.forget(long, 1);
            assert_eq!(
                fs.create(root, Name::new("ALONGF~1.TXT"), &SetAttr::new())
                    .unwrap_err()
                    .kind(),
                ErrorKind::AlreadyExists
            );
            fs.clear_cache();
            assert_eq!(fs.read_to_vec("REUSED.TXT").unwrap(), b"new");
            for i in (0..40).filter(|i| ![3, 4].contains(i)) {
                assert_eq!(fs.read_to_vec(&format!("F{i:07}.TXT")).unwrap(), [i as u8]);
            }
            let image = fs.unmount().unwrap().into_inner();
            common::assert_checks_clean(case, &image, "append planning");
        }
    }
}

#[cfg(feature = "async")]
#[test]
fn append_planning_recovers_cancelled_creation_at_every_await() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;

    let case = CASES[2];
    let image = common::blank(case);
    for budget in 0..100 {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(
            CacheOptions::new()
                .with_blocks(0)
                .with_directory_entries(64),
        );
        let root = fs.root();
        for i in 0..16 {
            let file = cancel::run_for(
                fs.create(root, Name::new(&format!("F{i:07}.TXT")), &SetAttr::new()),
                usize::MAX,
            )
            .unwrap()
            .unwrap();
            fs.forget(file, 1);
        }
        let result = cancel::run_for(
            fs.create(root, Name::new("NEW.TXT"), &SetAttr::new()),
            budget,
        );
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        let present = cancel::run_for(fs.lookup(root, Name::new("NEW.TXT")), usize::MAX)
            .unwrap()
            .is_ok();
        let created = cancel::run_for(
            fs.create(root, Name::new("new.txt"), &SetAttr::new()),
            usize::MAX,
        )
        .unwrap();
        if present {
            assert_eq!(created.unwrap_err().kind(), ErrorKind::AlreadyExists);
        } else {
            created.unwrap();
        }
        let image = cancel::run_for(fs.unmount(), usize::MAX)
            .unwrap()
            .unwrap()
            .0
            .into_inner();
        common::assert_checks_clean(case, &image, &format!("cancelled append budget {budget}"));
        if result.is_some() {
            return;
        }
    }
    panic!("append creation never completed");
}

#[test]
fn append_planning_honours_fixed_root_limits() {
    for case in CASES[..2].iter().copied() {
        let mut fs = common::formatted(case, hadris_fat::FatOptions::new().with_root_entries(16))
            .with_cache(CacheOptions::new().with_directory_entries(64));
        let root = fs.root();
        for i in 0..16 {
            let file = fs
                .create(root, Name::new(&format!("F{i:07}.TXT")), &SetAttr::new())
                .unwrap();
            fs.forget(file, 1);
        }
        assert_eq!(
            fs.create(root, Name::new("FULL.TXT"), &SetAttr::new())
                .unwrap_err()
                .kind(),
            ErrorKind::NoSpace
        );
        assert_eq!(
            fs.create(root, Name::new("f0000000.txt"), &SetAttr::new())
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
        fs.unlink(root, Name::new("F0000000.TXT")).unwrap();
        let file = fs
            .create(root, Name::new("REUSED.TXT"), &SetAttr::new())
            .unwrap();
        fs.forget(file, 1);
        let image = fs.unmount().unwrap().into_inner();
        common::assert_checks_clean(case, &image, "fixed root append limit");
    }
}

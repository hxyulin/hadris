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

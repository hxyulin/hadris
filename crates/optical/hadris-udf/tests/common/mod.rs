#![allow(dead_code)]

use hadris_fs::tree::{Content, Tree};
use hadris_fs::{DateTime, DeviceKind, DeviceNumber, FileTimes, Mode, SetMetadata};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::UdfOptions;

pub const SECTOR: BlockSize = BlockSize::new(2048).unwrap();

pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(31).wrapping_add(seed) % 251) as u8)
        .collect()
}

/// Files of every size class, nested and empty directories, names that
/// need 8-bit and 16-bit CS0, symlinks and a hard link.
pub fn sample() -> Tree {
    let mut tree = Tree::new();
    tree.add_file("readme.txt", Content::bytes("hello"))
        .unwrap();
    tree.add_file("empty.txt", Content::empty()).unwrap();
    tree.add_file("caf\u{e9}.txt", Content::bytes("latin1"))
        .unwrap();
    tree.add_file("emoji-\u{1F600}.bin", Content::bytes("wide"))
        .unwrap();
    tree.add_file("docs/big.bin", Content::bytes(pattern(70_000, 1)))
        .unwrap();
    tree.add_file("docs/sub/deep.txt", Content::bytes("deep"))
        .unwrap();
    tree.add_dir("emptydir").unwrap();
    for i in 0..80 {
        tree.add_file(
            &format!("many/file-{i:03}.txt"),
            Content::bytes(format!("file {i}")),
        )
        .unwrap();
    }
    tree.add_symlink("abs", "/docs/sub/deep.txt").unwrap();
    tree.add_symlink("docs/rel", "../readme.txt").unwrap();
    tree.add_hard_link("docs/link.txt", "readme.txt").unwrap();
    tree
}

pub fn time(seconds: i64) -> DateTime {
    DateTime::from_unix_seconds(seconds).unwrap()
}

pub fn with_metadata(tree: &mut Tree) {
    tree.set_metadata(
        "readme.txt",
        SetMetadata::new()
            .with_mode(Mode::new(0o4751))
            .with_uid(1000)
            .with_gid(100)
            .with_times(
                FileTimes::new()
                    .with_modified(time(1_700_000_000))
                    .with_accessed(time(1_700_000_100))
                    .with_changed(time(1_700_000_200)),
            ),
    )
    .unwrap();
    tree.set_metadata("docs", SetMetadata::new().with_mode(Mode::new(0o750)))
        .unwrap();
    tree.add_device("dev/null", DeviceKind::Char, DeviceNumber::new(1, 3))
        .unwrap();
}

/// Writes `tree` with `options` to a device of exactly the planned size.
pub fn image<C: hadris_fs::Clock>(tree: &Tree, options: &UdfOptions<C>) -> Vec<u8> {
    let report = hadris_udf::sync::plan(tree, options).unwrap();
    let mut dev = MemDevice::new(vec![0xAAu8; report.size_bytes() as usize], SECTOR);
    let written = hadris_udf::sync::write(&mut dev, tree, options).unwrap();
    assert_eq!(written, report);
    dev.into_inner()
}

pub fn open(bytes: Vec<u8>) -> hadris_udf::sync::UdfFs<MemDevice<Vec<u8>>> {
    hadris_udf::sync::UdfFs::open(MemDevice::new(bytes, SECTOR)).unwrap()
}

/// Recomputes the tag of the descriptor at `sector` over `crc_length`
/// bytes, keeping its identifier, version and location.
pub fn reseal(bytes: &mut [u8], sector: u64, crc_length: usize) {
    let at = sector as usize * 2048;
    let tag = hadris_udf::raw::Tag::read(&bytes[at..]).unwrap();
    hadris_udf::raw::Tag::seal(
        &mut bytes[at..at + 2048],
        tag.identifier.get(),
        tag.version.get(),
        tag.location.get(),
        crc_length,
    );
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    use core::task::{Context, Poll, Waker};
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}

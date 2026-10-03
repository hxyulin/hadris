#![no_main]
//! Fuzz the APFS reader: open an arbitrary container, then walk every volume's
//! directories, read every file and resolve every symlink. Arbitrary bytes
//! must never panic/abort/OOM.
//!
//! Self-consistency oracle (failures are tagged `ORACLE:`): every file is read
//! twice and the bytes must match.

use libfuzzer_sys::fuzz_target;

use hadris_apfs::sync::Container;
use hadris_apfs::types::filesystem::{
    parse_owned_directory_entries, stream_size, DT_DIR, DT_LNK, INODE_ROOT_DIRECTORY,
};
use hadris_apfs::types::{FileExtentRecord, InodeRecord, OwnedEntry};
use hadris_io::Cursor;
use hadris_storage::sync::StreamDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

type Apfs<'a> = Container<StreamDevice<hadris_storage::ReadOnly<Cursor<'a>>>>;

/// Total file bytes read per input, including zero-filled holes.
const BYTE_BUDGET: usize = 64 * 1024 * 1024;
/// Files and symlinks also read through the library lookups, which walk the
/// whole tree each time.
const FULL_READS: u32 = 16;

/// Reads a file from the volume's tree, already walked into `tree`, in
/// chunks charged to `bytes`. The size is fuzz-controlled, so no allocation
/// is sized from it.
fn read_pass(
    container: &mut Apfs<'_>,
    tree: &[OwnedEntry],
    inode: u64,
    bytes: &mut usize,
) -> Option<Vec<u8>> {
    let record = tree.iter().find_map(|entry| {
        InodeRecord::parse(&entry.key, &entry.value)
            .ok()
            .filter(|record| record.id == inode)
    })?;
    let mut extents: Vec<FileExtentRecord> = tree
        .iter()
        .filter_map(|entry| FileExtentRecord::parse(&entry.key, &entry.value).ok())
        .filter(|extent| extent.id == record.private_id)
        .collect();
    extents.sort_by_key(|extent| extent.logical_address);
    let size = stream_size(&record, &extents);
    let mut buf = [0u8; 64 * 1024];
    let mut out = Vec::new();
    while *bytes > 0 {
        let want = buf.len().min(*bytes);
        let n = container
            .read_extents_at(&extents, size, out.len() as u64, &mut buf[..want])
            .ok()?;
        if n == 0 {
            break;
        }
        *bytes -= n;
        out.extend_from_slice(&buf[..n]);
    }
    Some(out)
}

fn drive(data: &[u8]) {
    let geometry = BlockGeometry::new(
        BlockSize::new(512).unwrap(),
        BlockCount::new(data.len() as u64 / 512),
    );
    let Ok(mut container) = Container::open(StreamDevice::with_block_count(hadris_storage::ReadOnly::new(Cursor::new(data)), geometry.logical_block_size(), geometry.block_count().get()))
    else {
        return;
    };
    let Ok(latest) = container.latest_superblock() else {
        return;
    };
    let Ok(volumes) = container.volume_superblocks(&latest) else {
        return;
    };

    // A corrupt directory graph can fan out without bound, so cap the total
    // entries processed on any input. Each volume's tree is walked once.
    let mut budget: u32 = 2_000;
    let mut bytes = BYTE_BUDGET;
    let mut full_reads = FULL_READS;
    for volume in volumes.iter().take(4) {
        let Ok(tree) = container.filesystem_root_owned_entries(volume) else {
            continue;
        };
        let mut stack = vec![(INODE_ROOT_DIRECTORY, 0u32)];
        while let Some((dir, depth)) = stack.pop() {
            if depth > 32 {
                continue;
            }
            for entry in parse_owned_directory_entries(tree.iter().cloned(), dir) {
                if budget == 0 || bytes == 0 {
                    return;
                }
                budget -= 1;
                match entry.file_type() {
                    DT_DIR => stack.push((entry.file_id, depth + 1)),
                    DT_LNK if full_reads > 0 => {
                        full_reads -= 1;
                        let _ = container.symlink_target(volume, entry.file_id);
                    }
                    DT_LNK => {}
                    _ => {
                        if full_reads > 0 {
                            full_reads -= 1;
                            if let Ok(head) = container.read_file(volume, entry.file_id, 4096) {
                                bytes = bytes.saturating_sub(head.len());
                            }
                        }
                        let half = bytes / 2;
                        let mut first_budget = half;
                        let mut second_budget = half;
                        let first =
                            read_pass(&mut container, &tree, entry.file_id, &mut first_budget);
                        let second =
                            read_pass(&mut container, &tree, entry.file_id, &mut second_budget);
                        bytes -= (half - first_budget) + (half - second_budget);
                        assert_eq!(
                            first, second,
                            "ORACLE: repeated reads of {:?} disagree",
                            entry.name
                        );
                    }
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});

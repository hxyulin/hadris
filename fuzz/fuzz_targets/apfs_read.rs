#![no_main]
//! Fuzz the APFS reader: open an arbitrary container, then walk every volume's
//! directories, read every file and resolve every symlink. Arbitrary bytes
//! must never panic/abort/OOM.
//!
//! Self-consistency oracle (failures are tagged `ORACLE:`): every file is read
//! twice and the bytes must match.

use libfuzzer_sys::fuzz_target;

use hadris_apfs::sync::Container;
use hadris_apfs::types::filesystem::{stream_size, INODE_ROOT_DIRECTORY};
use hadris_apfs::types::VolumeSuperblock;
use hadris_io::Cursor;
use hadris_storage::sync::SeekBlockDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

type Apfs<'a> = Container<SeekBlockDevice<Cursor<'a>>>;

// The size is fuzz-controlled, so read in chunks with a byte cap instead of
// sizing an allocation from it.
fn read_pass(container: &mut Apfs<'_>, volume: &VolumeSuperblock, inode: u64) -> Option<Vec<u8>> {
    let record = container.inode_record(volume, inode).ok()??;
    let extents = container.file_extents(volume, record.private_id).ok()?;
    let size = stream_size(&record, &extents);
    let mut buf = [0u8; 64 * 1024];
    let mut out = Vec::new();
    while out.len() < 16 * 1024 * 1024 {
        let n = container
            .read_extents_at(&extents, size, out.len() as u64, &mut buf)
            .ok()?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Some(out)
}

fn drive(data: &[u8]) {
    let geometry = BlockGeometry::new(
        BlockSize::new(512).unwrap(),
        BlockCount(data.len() as u64 / 512),
    );
    let Ok(mut container) = Container::open(SeekBlockDevice::new(Cursor::new(data), geometry))
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
    // entries processed on any input.
    let mut budget: u32 = 2_000;
    for volume in volumes.iter().take(4) {
        let mut stack = vec![(INODE_ROOT_DIRECTORY, 0u32)];
        while let Some((dir, depth)) = stack.pop() {
            if depth > 32 {
                continue;
            }
            let Ok(entries) = container.directory_owned_entries(volume, dir) else {
                continue;
            };
            for entry in entries {
                if budget == 0 {
                    return;
                }
                budget -= 1;
                match entry.flags & 0xff {
                    4 => stack.push((entry.file_id, depth + 1)),
                    10 => {
                        let _ = container.symlink_target(volume, entry.file_id);
                    }
                    _ => {
                        let _ = container.read_file(volume, entry.file_id, 4096);
                        let first = read_pass(&mut container, volume, entry.file_id);
                        let second = read_pass(&mut container, volume, entry.file_id);
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

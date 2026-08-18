#![no_main]
//! Fuzz the NTFS reader: mount an arbitrary image, then walk every directory
//! and read every file. Arbitrary bytes must never panic/abort/OOM.
//!
//! Self-consistency oracle (failures are tagged `ORACLE:`): every file is
//! read twice through fresh readers and the bytes must match.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_ntfs::sync::{NtfsEntry, NtfsFs, NtfsFsReadExt};

fn drive(data: &[u8]) {
    let Ok(fs) = NtfsFs::open(Cursor::new(data)) else {
        return;
    };

    // Chunked read with a byte cap: the $DATA size is a fuzz-controlled u64,
    // so never size an allocation from it (`read_to_vec` would try to).
    let read_pass = |entry: &NtfsEntry| -> Option<Vec<u8>> {
        let mut reader = fs.read_file(entry).ok()?;
        let mut buf = [0u8; 64 * 1024];
        let mut out = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.len() >= 16 * 1024 * 1024 {
                        break;
                    }
                }
            }
        }
        Some(out)
    };

    // Depth-guarded worklist with a flat work budget: same rationale as
    // fat_read — a corrupt directory graph fans out exponentially, so bound
    // total entries processed on ANY input.
    let mut budget: u32 = 200_000;
    let mut stack = vec![(fs.root_dir(), 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let Ok(entries) = dir.entries() else { continue };
        for entry in entries {
            if budget == 0 {
                return;
            }
            budget -= 1;
            if entry.is_directory() {
                if let Ok(child) = dir.open_dir(entry.name()) {
                    stack.push((child, depth + 1));
                }
            } else {
                // Read-twice oracle: two fresh readers must yield identical bytes.
                let first = read_pass(&entry);
                let second = read_pass(&entry);
                assert_eq!(
                    first.is_some(),
                    second.is_some(),
                    "ORACLE: repeated reads of {:?} disagree on success",
                    entry.name()
                );
                if let (Some(a), Some(b)) = (first, second) {
                    assert_eq!(
                        a,
                        b,
                        "ORACLE: repeated reads of {:?} returned different bytes",
                        entry.name()
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});

#![no_main]
//! Fuzz the FAT reader: mount an arbitrary image, then walk every directory
//! and read every file. Arbitrary bytes must never panic/abort/OOM.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_fat::{FatFs, FatFsReadExt};

fn drive(data: &[u8]) {
    let Ok(fs) = FatFs::open(Cursor::new(data)) else {
        return;
    };

    // Depth-guarded worklist. The depth cap alone is NOT enough: a corrupt
    // directory graph (entries pointing at sibling/ancestor clusters) has a
    // path count that grows like branching^depth, so a naive walk fans out to
    // billions of `open_entry`/`read_file` calls and hangs — a harness DoS, not
    // a library bug (the library bounds single chains via ClusterLoop). A flat
    // work budget bounds total entries processed on ANY input.
    // ponytail: budget over visited-set — no per-format cluster accessor needed.
    let mut budget: u32 = 200_000;
    let mut stack = vec![(fs.root_dir(), 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        for item in dir.entries() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let Ok(de) = item else { continue };
            let Some(fe) = de.as_entry() else { continue };
            let name = fe.name();
            if name == "." || name == ".." {
                continue;
            }
            if fe.is_directory() {
                if let Ok(child) = dir.open_entry(fe) {
                    stack.push((child, depth + 1));
                }
            } else if let Ok(mut reader) = fs.read_file(fe) {
                let _ = reader.read_to_vec();
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});

#![no_main]
//! Fuzz the FAT *write* path: format a small in-memory FAT16 volume, then
//! apply a fuzz-driven sequence of create-dir / create+write-file / delete /
//! rename operations, remount fresh, and verify that the on-disk result
//! matches a shadow model of every successful operation.
//!
//! Invariants (all asserted with an "ORACLE:" prefix):
//! - the library never panics, aborts, or hangs on any op sequence;
//! - after remount, every walked entry is one the model expects (no phantom
//!   or missing entries) and every walked file reads back the exact bytes
//!   that were written, with the size recorded at write time.
//!
//! Individual op errors (AlreadyExists, DirectoryFull, NoSpace, ...) are
//! expected and simply skip the op; only model/filesystem divergence asserts.

use std::collections::HashMap;
use std::io::Cursor;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::{FatVolume, FatVolumeReadExt, FatVolumeWriteExt, FileEntry};
use libfuzzer_sys::fuzz_target;

/// FAT16 needs >= 4 MiB (hadris-fat format::calc::MIN_FAT16_SIZE).
const IMAGE_SIZE: usize = 4 * 1024 * 1024;
const MAX_OPS: usize = 64;
const MAX_FILE_SIZE: usize = 64 * 1024;
const MAX_TOTAL_WRITTEN: usize = 1024 * 1024;
/// Same flat-work budget pattern as fat_read.rs.
const WALK_BUDGET: u32 = 200_000;
const MAX_DEPTH: u32 = 64;

/// Deterministic byte source over the fuzz input; yields 0 once exhausted so
/// short inputs still produce well-defined (if degenerate) op streams.
struct Input<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Input<'a> {
    fn u8(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        b
    }

    fn u64(&mut self) -> u64 {
        let mut v = 0u64;
        for _ in 0..8 {
            v = (v << 8) | self.u8() as u64;
        }
        v
    }

    /// Build a valid 8.3 name: 1-8 chars base, optional 1-3 char extension,
    /// charset [A-Z0-9]. Guaranteed to round-trip without LFN entries.
    fn name(&mut self) -> String {
        let base_len = 1 + (self.u8() % 8) as usize;
        let mut name = String::with_capacity(base_len + 4);
        for _ in 0..base_len {
            name.push(name_char(self.u8()));
        }
        if self.u8() & 1 == 1 {
            let ext_len = 1 + (self.u8() % 3) as usize;
            name.push('.');
            for _ in 0..ext_len {
                name.push(name_char(self.u8()));
            }
        }
        name
    }
}

fn name_char(b: u8) -> char {
    (match b % 36 {
        n @ 0..=25 => b'A' + n,
        n => b'0' + (n - 26),
    }) as char
}

/// Expected file content: a deterministic function of the seed chosen when
/// the file was written (xorshift-mixed counter — no external PRNG crate).
fn pattern(seed: u64, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut x = seed | 1;
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// One live entry in the shadow model. `path` is the absolute path using '/'
/// separators with no leading component for the root (root child: "/NAME").
struct ModelEntry {
    path: String,
    entry: FileEntry,
    is_dir: bool,
    seed: u64,
    size: usize,
}

fn drive(data: &[u8]) {
    let mut image = vec![0u8; IMAGE_SIZE];
    let mut model: Vec<ModelEntry> = Vec::new();
    let mut total_written = 0usize;

    {
        let cursor = Cursor::new(&mut image[..]);
        let options = FatFormatOptions::new(IMAGE_SIZE as u64)
            .volume_label("FUZZ")
            .fat_type(FatTypeSelection::Fat16);
        let Ok(fs) = FatVolumeFormatter::format(cursor, options) else {
            // Formatting a fixed valid geometry must succeed; if it ever
            // fails that is itself a finding, but it is not fuzz-driven, so
            // just bail rather than assert.
            return;
        };

        let mut input = Input { data, pos: 0 };

        for _ in 0..MAX_OPS {
            let op = input.u8() % 4;
            // Selectable directories: the root plus every model dir. Used as
            // the parent for creates and the destination for renames.
            let mut dir_choices: Vec<Option<usize>> = vec![None];
            for (i, m) in model.iter().enumerate() {
                if m.is_dir {
                    dir_choices.push(Some(i));
                }
            }
            let dir_sel = dir_choices[(input.u8() as usize) % dir_choices.len()];

            if op == 2 {
                // delete entry
                if model.is_empty() {
                    continue;
                }
                let idx = (input.u8() as usize) % model.len();
                // Deleting a non-empty dir fails inside the library (the
                // model mirrors the disk, so model children imply on-disk
                // children); failed ops leave both sides unchanged.
                if fs.delete(&model[idx].entry).is_ok() {
                    model.remove(idx);
                }
                continue;
            }

            let (parent_path, parent_dir) = match dir_sel {
                None => (String::new(), fs.root_dir()),
                Some(i) => {
                    let Ok(d) = fs.open_dir_entry(&model[i].entry) else {
                        continue;
                    };
                    (model[i].path.clone(), d)
                }
            };

            match op {
                0 => {
                    // create dir
                    let name = input.name();
                    if name_in_use(&model, &parent_path, &name) {
                        continue;
                    }
                    if fs.create_dir(&parent_dir, &name).is_err() {
                        continue;
                    }
                    // Recover the FileEntry (create_dir returns a FatDir).
                    let Ok(Some(fe)) = parent_dir.find(&name) else {
                        // The dir was just created; a failed lookup desyncs
                        // the model, so stop mutating but still verify below.
                        break;
                    };
                    let path = format!("{parent_path}/{}", fe.name());
                    model.push(ModelEntry {
                        path,
                        entry: fe,
                        is_dir: true,
                        seed: 0,
                        size: 0,
                    });
                }
                1 => {
                    // create + write file
                    let name = input.name();
                    if name_in_use(&model, &parent_path, &name) {
                        continue;
                    }
                    let want = (input.u8() as usize) | ((input.u8() as usize) << 8);
                    let size =
                        (want % MAX_FILE_SIZE).min(MAX_TOTAL_WRITTEN.saturating_sub(total_written));
                    let seed = input.u64();
                    let content = pattern(seed, size);

                    let Ok(fe) = fs.create_file(&parent_dir, &name) else {
                        continue;
                    };
                    let ok = (|| {
                        let mut w = fs.write_file(&fe)?;
                        w.write(&content)?;
                        w.finish()
                    })();
                    if ok.is_err() {
                        // Partial op: the entry exists on disk but its content
                        // is unknown. Best-effort cleanup; if even that fails
                        // the model can no longer mirror the disk, so stop.
                        if fs.delete(&fe).is_err() {
                            break;
                        }
                        continue;
                    }
                    total_written += size;
                    let path = format!("{parent_path}/{}", fe.name());
                    model.push(ModelEntry {
                        path,
                        entry: fe,
                        is_dir: false,
                        seed,
                        size,
                    });
                }
                2 => unreachable!("handled above"),
                _ => {
                    // rename (same-parent or move)
                    if model.is_empty() {
                        continue;
                    }
                    let idx = (input.u8() as usize) % model.len();
                    let name = input.name();
                    if name_in_use(&model, &parent_path, &name) {
                        continue;
                    }
                    let old_path = model[idx].path.clone();
                    // Never move a directory into itself or its own subtree:
                    // that would orphan the subtree on disk while the model
                    // still tracks it — a cycle, not a library bug.
                    if model[idx].is_dir
                        && (parent_path == old_path
                            || parent_path.starts_with(&format!("{old_path}/")))
                    {
                        continue;
                    }
                    let Ok(new_fe) = fs.rename(&model[idx].entry, &parent_dir, &name) else {
                        continue;
                    };
                    let new_path = format!("{parent_path}/{}", new_fe.name());
                    if model[idx].is_dir {
                        let prefix = format!("{old_path}/");
                        for m in model.iter_mut() {
                            if let Some(rest) = m.path.strip_prefix(&prefix) {
                                m.path = format!("{new_path}/{rest}");
                            }
                        }
                    }
                    model[idx].path = new_path;
                    model[idx].entry = new_fe;
                }
            }
        }
    }

    // Remount fresh and walk the whole tree with the same bounded-worklist
    // pattern as fat_read.rs.
    let Ok(fs) = FatVolume::open(Cursor::new(&image[..])) else {
        assert!(
            model.is_empty(),
            "ORACLE: remount failed with a non-empty model"
        );
        return;
    };

    let expected_files: HashMap<&str, (u64, usize)> = model
        .iter()
        .filter(|m| !m.is_dir)
        .map(|m| (m.path.as_str(), (m.seed, m.size)))
        .collect();
    let expected_dirs: Vec<&str> = model
        .iter()
        .filter(|m| m.is_dir)
        .map(|m| m.path.as_str())
        .collect();

    let mut seen: Vec<String> = Vec::new();

    let mut budget = WALK_BUDGET;
    let mut stack = vec![(fs.root_dir(), String::new(), 0u32)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
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
            assert!(
                !name.is_empty(),
                "ORACLE: entry with empty name under {prefix}"
            );
            let path = format!("{prefix}/{name}");
            if fe.is_directory() {
                assert!(
                    expected_dirs.contains(&path.as_str()),
                    "ORACLE: unexpected directory {path} after remount"
                );
                seen.push(path.clone());
                if let Ok(child) = dir.open_entry(fe) {
                    stack.push((child, path, depth + 1));
                }
            } else {
                let Some(&(seed, size)) = expected_files.get(path.as_str()) else {
                    panic!("ORACLE: unexpected file {path} after remount");
                };
                assert_eq!(
                    fe.len() as usize,
                    size,
                    "ORACLE: size mismatch for {path}: on-disk {} vs written {size}",
                    fe.len()
                );
                let Ok(mut reader) = fs.read_file(fe) else {
                    panic!("ORACLE: cannot open expected file {path} after remount");
                };
                let Ok(bytes) = reader.read_to_vec() else {
                    panic!("ORACLE: cannot read expected file {path} after remount");
                };
                assert!(
                    bytes == pattern(seed, size),
                    "ORACLE: content mismatch for {path} after remount"
                );
                seen.push(path);
            }
        }
    }

    for m in &model {
        assert!(
            seen.iter().any(|p| p == &m.path),
            "ORACLE: expected entry {} missing after remount",
            m.path
        );
    }
}

fn name_in_use(model: &[ModelEntry], parent_path: &str, name: &str) -> bool {
    let prefix = format!("{parent_path}/");
    model.iter().any(|m| {
        m.path
            .strip_prefix(&prefix)
            .is_some_and(|rest| !rest.contains('/') && rest.eq_ignore_ascii_case(name))
    })
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});

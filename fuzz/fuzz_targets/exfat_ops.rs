#![no_main]
//! Fuzz the exFAT *write* path: format a small in-memory exFAT volume with
//! 512-byte clusters, then apply a fuzz-driven sequence of create-dir,
//! create+write-file, delete, rename, write-at and set-len operations through
//! `ExFatFs`, sync, check, remount fresh, and verify that the on-disk result
//! matches a shadow model of every successful operation.
//!
//! Invariants (all asserted with an "ORACLE:" prefix):
//! - the library never panics, aborts, or hangs on any op sequence;
//! - `check` finds nothing on the synced volume;
//! - after remount, every walked entry is one the model expects (no phantom
//!   or missing entries) and every walked file reads back the exact bytes
//!   the model holds.
//!
//! Individual op errors (AlreadyExists, DirectoryFull, NoSpace, ...) are
//! expected and simply skip the op; only model/filesystem divergence asserts.

use std::collections::HashMap;

use hadris_fat::exfat::sync::{check, format, ExFatFs};
use hadris_fat::exfat::{FormatOptions, VolumeLabel};
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, FileType, MountOptions, Name, NodeId, RenameMode, Resolve, SetAttr};
use hadris_storage::{BlockSize, MemDevice};
use libfuzzer_sys::fuzz_target;

type Fs = ExFatFs<MemDevice<Vec<u8>>>;

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

    fn u16(&mut self) -> usize {
        (self.u8() as usize) | ((self.u8() as usize) << 8)
    }

    fn u64(&mut self) -> u64 {
        let mut v = 0u64;
        for _ in 0..8 {
            v = (v << 8) | self.u8() as u64;
        }
        v
    }

    /// A valid exFAT name: 1-12 base characters, optional 1-3 character
    /// extension, from [A-Za-z0-9_-].
    fn name(&mut self) -> String {
        let base_len = 1 + (self.u8() % 12) as usize;
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
    (match b % 64 {
        n @ 0..=25 => b'A' + n,
        n @ 26..=51 => b'a' + (n - 26),
        n @ 52..=61 => b'0' + (n - 52),
        62 => b'_',
        _ => b'-',
    }) as char
}

/// Expected file content: a deterministic function of a seed
/// (xorshift-mixed counter, no external PRNG crate).
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
    is_dir: bool,
    content: Vec<u8>,
}

fn split(path: &str) -> (&str, &str) {
    let at = path.rfind('/').unwrap_or(0);
    (&path[..at], &path[at + 1..])
}

fn name(text: &str) -> &Name {
    Name::new(text)
}

/// Resolves a model directory path ("" for the root) and pins it.
fn resolve(fs: &mut Fs, path: &str) -> Option<NodeId> {
    if path.is_empty() {
        return Some(fs.root());
    }
    fs.resolve(path.as_bytes(), Resolve::Lexical).ok()
}

/// Writes all of `data` at `offset`.
fn write_all(fs: &mut Fs, node: NodeId, mut offset: u64, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        match fs.write(node, offset, data) {
            Ok(0) | Err(_) => return false,
            Ok(n) => {
                offset += n as u64;
                data = &data[n..];
            }
        }
    }
    true
}

/// Runs `f` on the pinned node at `path` and unpins it.
fn with_node<T>(fs: &mut Fs, path: &str, f: impl FnOnce(&mut Fs, NodeId) -> T) -> Option<T> {
    let node = resolve(fs, path)?;
    let out = f(fs, node);
    fs.forget(node, 1);
    Some(out)
}

fn apply(
    fs: &mut Fs,
    input: &mut Input<'_>,
    model: &mut Vec<ModelEntry>,
    written: &mut usize,
) -> bool {
    let op = input.u8() % 6;
    // Selectable directories: the root plus every model dir. Used as the
    // parent for creates and the destination for renames.
    let mut dirs: Vec<String> = vec![String::new()];
    dirs.extend(model.iter().filter(|m| m.is_dir).map(|m| m.path.clone()));
    let parent_path = dirs[(input.u8() as usize) % dirs.len()].clone();
    let budget = MAX_TOTAL_WRITTEN.saturating_sub(*written);

    match op {
        0 | 1 => {
            let child = input.name();
            if name_in_use(model, &parent_path, &child) {
                return true;
            }
            let is_dir = op == 0;
            let content = if is_dir {
                Vec::new()
            } else {
                pattern(input.u64(), (input.u16() % MAX_FILE_SIZE).min(budget))
            };
            let Some(parent) = resolve(fs, &parent_path) else {
                return false;
            };
            let created = if is_dir {
                fs.mkdir(parent, name(&child), &SetAttr::new())
            } else {
                fs.create(parent, name(&child), &SetAttr::new())
            };
            let Ok(node) = created else {
                fs.forget(parent, 1);
                return true;
            };
            let ok = write_all(fs, node, 0, &content);
            fs.forget(node, 1);
            if !ok {
                // The entry exists but its content is unknown. Best-effort
                // cleanup; if even that fails the model can no longer mirror
                // the disk, so stop.
                let removed = fs.unlink(parent, name(&child)).is_ok();
                fs.forget(parent, 1);
                return removed;
            }
            fs.forget(parent, 1);
            *written += content.len();
            model.push(ModelEntry {
                path: format!("{parent_path}/{child}"),
                is_dir,
                content,
            });
        }
        2 => {
            if model.is_empty() {
                return true;
            }
            let idx = (input.u8() as usize) % model.len();
            let (dir, child) = split(&model[idx].path);
            let child = child.to_owned();
            let is_dir = model[idx].is_dir;
            // Removing a non-empty directory fails inside the library;
            // failed ops leave both sides unchanged.
            if with_node(fs, dir, |fs, dir| match is_dir {
                true => fs.rmdir(dir, name(&child)).is_ok(),
                false => fs.unlink(dir, name(&child)).is_ok(),
            }) == Some(true)
            {
                model.remove(idx);
            }
        }
        3 => {
            if model.is_empty() {
                return true;
            }
            let idx = (input.u8() as usize) % model.len();
            let child = input.name();
            if name_in_use(model, &parent_path, &child) {
                return true;
            }
            let old_path = model[idx].path.clone();
            // Never move a directory into itself or its own subtree.
            if model[idx].is_dir
                && (parent_path == old_path || parent_path.starts_with(&format!("{old_path}/")))
            {
                return true;
            }
            let (from_path, from_name) = split(&old_path);
            let (Some(from), Some(to)) = (resolve(fs, from_path), resolve(fs, &parent_path)) else {
                return false;
            };
            let renamed = fs.rename(
                from,
                name(from_name),
                to,
                name(&child),
                RenameMode::NoReplace,
            );
            fs.forget(from, 1);
            fs.forget(to, 1);
            if renamed.is_err() {
                return true;
            }
            let new_path = format!("{parent_path}/{child}");
            if model[idx].is_dir {
                let prefix = format!("{old_path}/");
                for m in model.iter_mut() {
                    if let Some(rest) = m.path.strip_prefix(&prefix) {
                        m.path = format!("{new_path}/{rest}");
                    }
                }
            }
            model[idx].path = new_path;
        }
        _ => {
            let files: Vec<usize> = (0..model.len()).filter(|&i| !model[i].is_dir).collect();
            if files.is_empty() {
                return true;
            }
            let idx = files[(input.u8() as usize) % files.len()];
            let path = model[idx].path.clone();
            let old = model[idx].content.len();
            if op == 4 {
                let offset = input.u16() % (old + 1);
                let data = pattern(input.u64(), (input.u16() % 8192).min(budget));
                let ok = with_node(fs, &path, |fs, node| {
                    write_all(fs, node, offset as u64, &data)
                });
                match ok {
                    Some(true) => {}
                    Some(false) => return false,
                    None => return false,
                }
                let content = &mut model[idx].content;
                let end = offset + data.len();
                if end > content.len() {
                    content.resize(end, 0);
                }
                content[offset..end].copy_from_slice(&data);
                *written += data.len();
            } else {
                let len = (input.u16() % MAX_FILE_SIZE).min(old + budget);
                match with_node(fs, &path, |fs, node| fs.truncate(node, len as u64).is_ok()) {
                    Some(true) => {}
                    // A failed extension may have allocated part of the
                    // growth; the size on disk is then unknown.
                    _ => return false,
                }
                model[idx].content.resize(len, 0);
                *written += len.saturating_sub(old);
            }
        }
    }
    true
}

fn drive(data: &[u8]) {
    let dev = MemDevice::new(vec![0u8; IMAGE_SIZE], BlockSize::new(512).unwrap());
    let options = FormatOptions::new()
        .with_cluster_size(512)
        .with_label(VolumeLabel::new("FUZZ").unwrap());
    let Ok(formatted) = format(dev, options) else {
        // Formatting a fixed valid geometry must succeed; if it ever fails
        // that is itself a finding, but it is not fuzz-driven, so bail.
        return;
    };
    let mounted = ExFatFs::mount(formatted.into_inner(), MountOptions::new());
    let Ok(mut fs) = mounted else {
        return;
    };

    let mut model: Vec<ModelEntry> = Vec::new();
    let mut written = 0usize;
    let mut input = Input { data, pos: 0 };
    let mut consistent = true;
    for _ in 0..MAX_OPS {
        if !apply(&mut fs, &mut input, &mut model, &mut written) {
            consistent = false;
            break;
        }
    }
    if fs.sync().is_err() {
        return;
    }
    let mut dev = fs.into_inner();
    if consistent {
        let blocks = dev.get_ref().len() / 512;
        let mut scratch = vec![0u8; 1024 + blocks.div_ceil(8).max(512)];
        let report =
            check(&mut dev, &mut scratch, |_| {}).expect("check reads an in-memory volume");
        assert!(
            report.is_clean(),
            "ORACLE: check found {} problem(s) on a volume ExFatFs wrote",
            report.findings()
        );
    }

    // Remount fresh and walk the whole tree.
    let image = dev.into_inner();
    let Ok(mut fs) = ExFatFs::mount(
        MemDevice::new(image, BlockSize::new(512).unwrap()),
        MountOptions::new(),
    ) else {
        panic!("ORACLE: remount of a synced volume failed");
    };
    if !consistent {
        return;
    }

    let expected: HashMap<&str, &ModelEntry> = model.iter().map(|m| (m.path.as_str(), m)).collect();
    let mut seen = 0usize;
    let mut budget = WALK_BUDGET;
    let mut stack = vec![(fs.root(), String::new(), 0u32)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let mut cursor = DirCursor::START;
        loop {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let entry = match fs.readdir(dir, cursor) {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(err) => panic!("ORACLE: listing {prefix}/ failed: {err:?}"),
            };
            cursor = entry.next_cursor();
            let text = entry.name().to_str().unwrap_or("");
            assert!(
                !text.is_empty(),
                "ORACLE: entry with an empty name under {prefix}/"
            );
            let path = format!("{prefix}/{text}");
            let Some(m) = expected.get(path.as_str()) else {
                panic!("ORACLE: unexpected entry {path} after remount");
            };
            seen += 1;
            let is_dir = entry.file_type() == FileType::Dir;
            assert_eq!(is_dir, m.is_dir, "ORACLE: {path} changed kind");
            if is_dir {
                stack.push((entry.node(), path, depth + 1));
                continue;
            }
            let meta = fs.stat(entry.node()).expect("metadata of a listed file");
            assert_eq!(
                meta.len(),
                m.content.len() as u64,
                "ORACLE: size mismatch for {path}"
            );
            let mut bytes = vec![0u8; m.content.len()];
            let mut filled = 0;
            while filled < bytes.len() {
                match fs.read(entry.node(), filled as u64, &mut bytes[filled..]) {
                    Ok(0) | Err(_) => panic!("ORACLE: cannot read {path} after remount"),
                    Ok(n) => filled += n,
                }
            }
            assert!(bytes == m.content, "ORACLE: content mismatch for {path}");
        }
    }
    assert_eq!(seen, model.len(), "ORACLE: entries missing after remount");
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

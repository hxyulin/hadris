use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use hadris_fs::{FileType, Node};

use super::names::Rules;
use super::susp::{SplitSu, SuBuilder, inline_space};
use super::{FileKind, PlanResult, Planner, SECTOR, TreeKind, invalid, too_large};
use crate::error::Detail;
use crate::raw::{DirDateTime, DirectoryRecord, FileFlags, SECTOR_SIZE, U16Both, U32Both};
use crate::rock_ridge::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFLNK, S_IFREG};

impl Planner<'_> {
    /// The records of `dir` in tree `ti`, sorted by identifier.
    pub(super) fn records(&self, dir: usize, ti: usize) -> PlanResult<Vec<PendingRecord>> {
        let (tree, rules) = self.trees[ti];
        let rr = self.rock_ridge && tree == TreeKind::Primary;
        let d = &self.dirs[dir];
        let mut records = Vec::new();
        let now = self.record_time(&d.meta);

        let dot = if rr {
            let mut b = SuBuilder::default();
            if dir == 0 {
                b.sp();
            }
            self.posix(
                &mut b,
                &d.meta,
                S_IFDIR,
                0o755,
                self.dir_links(dir),
                d.serial,
            );
            b.nm_current();
            if dir == 0 {
                b.er();
            }
            b.split(inline_space(1))
        } else {
            SplitSu::default()
        };
        records.push(PendingRecord {
            name: vec![0],
            split: dot,
            extent: self.dir_ref(dir, ti),
            flags: FileFlags::DIRECTORY,
            time: now,
        });

        let parent = self.parent_in(dir, ti);
        let dotdot = if rr {
            let p = &self.dirs[parent];
            let mut b = SuBuilder::default();
            self.posix(
                &mut b,
                &p.meta,
                S_IFDIR,
                0o755,
                self.dir_links(parent),
                p.serial,
            );
            b.nm_parent();
            if d.moved_to.is_some() {
                b.pl(self.dir_ref(d.parent, ti).0);
            }
            b.split(inline_space(1))
        } else {
            SplitSu::default()
        };
        records.push(PendingRecord {
            name: vec![1],
            split: dotdot,
            extent: self.dir_ref(parent, ti),
            flags: FileFlags::DIRECTORY,
            time: self.record_time(&self.dirs[parent].meta),
        });

        let mut children: Vec<(usize, bool)> = self
            .subdirs_in(dir, ti)
            .iter()
            .map(|&child| (child, false))
            .collect();
        if rr {
            children.extend(d.placeholders.iter().map(|&child| (child, true)));
        }
        for (child, placeholder) in children {
            let c = &self.dirs[child];
            let iso_name = if placeholder || tree != TreeKind::Primary {
                &c.name
            } else {
                &c.iso_name
            };
            let name = rules.directory(iso_name);
            let split = if rr {
                let mut b = SuBuilder::default();
                self.posix(
                    &mut b,
                    &c.meta,
                    S_IFDIR,
                    0o755,
                    self.dir_links(child),
                    c.serial,
                );
                b.nm(c.name.as_bytes());
                if placeholder {
                    b.cl(self.dir_ref(child, ti).0);
                } else if c.moved_to.is_some() {
                    b.re();
                }
                b.split(inline_space(name.len()))
            } else {
                SplitSu::default()
            };
            let flags = if placeholder {
                FileFlags::empty()
            } else {
                FileFlags::DIRECTORY
            };
            records.push(PendingRecord {
                name,
                split,
                extent: self.dir_ref(child, ti),
                flags,
                time: self.record_time(&c.meta),
            });
        }

        for &file in &d.files {
            let f = &self.files[file];
            if !rr && matches!(f.kind, FileKind::Symlink { .. } | FileKind::Device { .. }) {
                continue;
            }
            let name = rules.file(&f.name);
            let split = if rr {
                let mut b = SuBuilder::default();
                let (type_mode, default) = match f.kind {
                    FileKind::Symlink { .. } => (S_IFLNK, 0o777),
                    FileKind::Device {
                        kind: FileType::BlockDevice,
                        ..
                    } => (S_IFBLK, 0o600),
                    FileKind::Device { .. } => (S_IFCHR, 0o600),
                    _ => (S_IFREG, 0o644),
                };
                self.posix(
                    &mut b,
                    &f.meta,
                    type_mode,
                    default,
                    f.links,
                    self.file_serial(f),
                );
                b.nm(f.name.as_bytes());
                match f.kind {
                    FileKind::Symlink { .. } => {
                        if let Some(target) = self.tree.get(&f.path).and_then(Node::target) {
                            b.sl(target);
                        }
                    }
                    FileKind::Device { number, .. } => b.pn(number.major(), number.minor()),
                    _ => {}
                }
                b.split(inline_space(name.len()))
            } else {
                SplitSu::default()
            };
            let extents = self.file_extents(f);
            let time = self.record_time(&f.meta);
            let last = extents.len() - 1;
            for (index, (block, len)) in extents.into_iter().enumerate() {
                let flags = if index == last {
                    FileFlags::empty()
                } else {
                    FileFlags::NOT_FINAL
                };
                let len = u32::try_from(len).map_err(|_| too_large())?;
                records.push(PendingRecord {
                    name: name.clone(),
                    split: split.clone(),
                    extent: (block, len),
                    flags,
                    time,
                });
            }
        }

        dedup(&mut records, rules);
        records.sort_by(|a, b| {
            let rank = |name: &[u8]| match name {
                [0] => 0,
                [1] => 1,
                _ => 2,
            };
            rank(&a.name)
                .cmp(&rank(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(records)
    }
}

/// A directory record before it is written.
#[derive(Debug, Clone)]
pub(super) struct PendingRecord {
    pub(super) name: Vec<u8>,
    split: SplitSu,
    pub(super) extent: (u32, u32),
    pub(super) flags: FileFlags,
    time: DirDateTime,
}

impl PendingRecord {
    fn build(&self) -> PlanResult<DirectoryRecord> {
        let mut record = DirectoryRecord::new(&self.name, &self.split.inline)
            .ok_or(invalid(Detail::DirectoryRecord))?;
        let header = record.header_mut();
        header.extent = U32Both::new(self.extent.0);
        header.data_len = U32Both::new(self.extent.1);
        header.date_time = self.time;
        header.flags = self.flags.bits();
        header.volume_sequence_number = U16Both::new(1);
        Ok(record)
    }

    fn len(&self) -> u64 {
        let su_start = (33 + self.name.len() + 1) & !1;
        ((su_start + self.split.inline.len() + 1) & !1) as u64
    }
}

/// Gives names that repeat in a directory a `_n` suffix; the records of one
/// multi-extent file keep one name.
fn dedup(records: &mut [PendingRecord], rules: Rules) {
    let mut seen = BTreeSet::new();
    let mut start = 0;
    while start < records.len() {
        if matches!(records[start].name[..], [0] | [1]) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < records.len() && records[end - 1].flags.contains(FileFlags::NOT_FINAL) {
            end += 1;
        }
        let original = records[start].name.clone();
        let unique = if seen.contains(&original) {
            let mut n = 1;
            loop {
                let candidate = rules.dedup(&original, n);
                n += 1;
                if !seen.contains(&candidate) {
                    break candidate;
                }
            }
        } else {
            original
        };
        seen.insert(unique.clone());
        for record in &mut records[start..end] {
            record.name = unique.clone();
        }
        start = end;
    }
}

/// The offset of each continuation area of `records`, in order, from the
/// first continuation block, and the bytes they span: an area that would
/// cross a block boundary starts the next block.
pub(super) fn place_areas(records: &[PendingRecord]) -> (Vec<u64>, u64) {
    let mut places = Vec::new();
    let mut at = 0u64;
    for area in records.iter().flat_map(|r| &r.split.areas) {
        let len = area.len() as u64;
        if at % SECTOR + len > SECTOR {
            at = at.div_ceil(SECTOR) * SECTOR;
        }
        places.push(at);
        at += len;
    }
    (places, at)
}

/// The first sector and the sector count of `records` written from byte
/// `pos`: a record that does not fit a sector starts the next one.
pub(super) fn layout_records(pos: u64, records: &[PendingRecord]) -> (u64, u64) {
    let start = pos.div_ceil(SECTOR) * SECTOR;
    let mut at = start;
    for record in records {
        let len = record.len();
        let remaining = SECTOR - at % SECTOR;
        if len > remaining {
            at += remaining;
        }
        at += len;
    }
    let end = at.div_ceil(SECTOR) * SECTOR;
    (start / SECTOR, (end - start) / SECTOR)
}

/// The bytes of a directory at `block` of `size` bytes, followed by the
/// continuation area its `CE` entries point at.
pub(super) fn emit_records(
    block: u32,
    size: u32,
    records: &mut [PendingRecord],
) -> PlanResult<Vec<u8>> {
    let ca_block = block + size / SECTOR_SIZE as u32;
    let (places, _) = place_areas(records);
    let mut next = places.iter();
    for record in records.iter_mut() {
        let at: Vec<(u32, u32)> = next
            .by_ref()
            .take(record.split.areas.len())
            .map(|&at| (ca_block + (at / SECTOR) as u32, (at % SECTOR) as u32))
            .collect();
        record.split.patch_ce(&at);
    }
    let mut out = Vec::with_capacity(size as usize);
    for record in records.iter() {
        let bytes = record.build()?;
        let remaining = SECTOR_SIZE - out.len() % SECTOR_SIZE;
        if bytes.len() > remaining {
            out.resize(out.len() + remaining, 0);
        }
        out.extend_from_slice(bytes.as_bytes());
    }
    if out.len() > size as usize {
        return Err(invalid(Detail::DirectoryRecord));
    }
    out.resize(size as usize, 0);
    for (area, at) in records.iter().flat_map(|r| &r.split.areas).zip(places) {
        out.resize(size as usize + at as usize, 0);
        out.extend_from_slice(area);
    }
    Ok(out)
}

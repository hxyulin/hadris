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
        let (kind, rules) = self.trees[ti];
        let builder = DirectoryBuilder {
            planner: self,
            tree: ti,
            kind,
            rules,
            rock_ridge: self.rock_ridge && kind == TreeKind::Primary,
        };
        let d = &self.dirs[dir];
        let mut records = Vec::new();
        records.push(builder.special(dir, false));
        records.push(builder.special(dir, true));
        let placeholders = if builder.rock_ridge {
            &d.placeholders[..]
        } else {
            &[]
        };
        let children = self
            .subdirs_in(dir, ti)
            .iter()
            .map(|&child| (child, false))
            .chain(placeholders.iter().map(|&child| (child, true)));
        for (child, placeholder) in children {
            records.push(builder.child(child, placeholder));
        }
        for &file in &d.files {
            builder.append_file(file, &mut records)?;
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

struct DirectoryBuilder<'a, 'tree> {
    planner: &'a Planner<'tree>,
    tree: usize,
    kind: TreeKind,
    rules: Rules,
    rock_ridge: bool,
}

impl DirectoryBuilder<'_, '_> {
    fn directory_posix(&self, builder: &mut SuBuilder, dir: usize) {
        let d = &self.planner.dirs[dir];
        self.planner.posix(
            builder,
            &d.meta,
            S_IFDIR,
            0o755,
            self.planner.dir_links(dir),
            d.serial,
        );
    }

    fn special(&self, dir: usize, parent: bool) -> PendingRecord {
        let p = self.planner;
        let target = if parent {
            p.parent_in(dir, self.tree)
        } else {
            dir
        };
        let split = if self.rock_ridge {
            let mut b = SuBuilder::default();
            if !parent && dir == 0 {
                b.sp();
            }
            self.directory_posix(&mut b, target);
            if parent {
                b.nm_parent();
                if p.dirs[dir].moved_to.is_some() {
                    b.pl(p.dir_ref(p.dirs[dir].parent, self.tree).0);
                }
            } else {
                b.nm_current();
                if dir == 0 {
                    b.er();
                }
            }
            b.split(inline_space(1))
        } else {
            SplitSu::default()
        };
        PendingRecord {
            name: vec![u8::from(parent)],
            split,
            extent: p.dir_ref(target, self.tree),
            flags: FileFlags::DIRECTORY,
            time: p.record_time(&p.dirs[target].meta),
        }
    }

    fn child(&self, child: usize, placeholder: bool) -> PendingRecord {
        let p = self.planner;
        let c = &p.dirs[child];
        let source = if placeholder || self.kind != TreeKind::Primary {
            &c.name
        } else {
            &c.iso_name
        };
        let name = self.rules.directory(source);
        let split = if self.rock_ridge {
            let mut b = SuBuilder::default();
            self.directory_posix(&mut b, child);
            b.nm(c.name.as_bytes());
            if placeholder {
                b.cl(p.dir_ref(child, self.tree).0);
            } else if c.moved_to.is_some() {
                b.re();
            }
            b.split(inline_space(name.len()))
        } else {
            SplitSu::default()
        };
        PendingRecord {
            name,
            split,
            extent: p.dir_ref(child, self.tree),
            flags: if placeholder {
                FileFlags::empty()
            } else {
                FileFlags::DIRECTORY
            },
            time: p.record_time(&c.meta),
        }
    }

    fn append_file(&self, file: usize, records: &mut Vec<PendingRecord>) -> PlanResult<()> {
        let p = self.planner;
        let f = &p.files[file];
        if !self.rock_ridge && matches!(f.kind, FileKind::Symlink { .. } | FileKind::Device { .. })
        {
            return Ok(());
        }
        let name = self.rules.file(&f.name);
        let split = if self.rock_ridge {
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
            p.posix(
                &mut b,
                &f.meta,
                type_mode,
                default,
                f.links,
                p.file_serial(f),
            );
            b.nm(f.name.as_bytes());
            match f.kind {
                FileKind::Symlink { .. } => {
                    if let Some(target) = p.tree.get(&f.path).and_then(Node::target) {
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
        let extents = p.file_extents(f);
        let last = extents.len() - 1;
        let time = p.record_time(&f.meta);
        for (index, (block, len)) in extents.into_iter().enumerate() {
            records.push(PendingRecord {
                name: name.clone(),
                split: split.clone(),
                extent: (block, u32::try_from(len).map_err(|_| too_large())?),
                flags: if index == last {
                    FileFlags::empty()
                } else {
                    FileFlags::NOT_FINAL
                },
                time,
            });
        }
        Ok(())
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

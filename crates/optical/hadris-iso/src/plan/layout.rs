use super::directory::{emit_records, layout_records, place_areas};
use super::{
    BACKUP_GPT_SECTORS, CATALOG, FileKind, MAX_EXTENT, PADDING_BLOCKS, PathTableLocation, Plan,
    PlanResult, Planner, Region, SECTOR, block_of, too_large,
};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::options::{Hybrid, PartitionScheme};
use crate::raw::SECTOR_SIZE;

struct PlacedFile {
    block: u64,
    node: usize,
    path: String,
    len: u64,
}

impl Planner<'_> {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(target = "hadris::iso", level = "trace", skip_all)
    )]
    pub(super) fn layout(mut self) -> PlanResult<Plan> {
        let order = self.preorder();
        let files = self.file_order(&order);
        let desc_end = self.base.descriptors + self.descriptor_count();
        let mut cursor = desc_end
            .max(self.opts.min_blocks())
            .max(self.base.first_block)
            * SECTOR;

        let mut placed = self.prepare_file_extents(&files)?;
        self.place_directories(&order, &mut cursor)?;
        let file_regions = self.place_files(&files, &mut placed, &mut cursor)?;

        let mut regions = Vec::new();
        for partition in self.opts.hybrid().map_or(&[][..], Hybrid::appended) {
            let content = partition.content().clone();
            let block = cursor.div_ceil(SECTOR);
            self.appended
                .push((block_of(block * SECTOR)?, content.len()));
            cursor = block * SECTOR + content.len();
            regions.push(Region::Content {
                block,
                content,
                info: None,
            });
        }
        for &dir in &order {
            for ti in 0..self.trees.len() {
                if !self.has_dir(dir, ti) {
                    continue;
                }
                let (start, size) = self.dir_ref(dir, ti);
                let mut records = self.records(dir, ti)?;
                let data = emit_records(start, size, &mut records)?;
                regions.push(Region::Bytes {
                    block: u64::from(start),
                    data,
                });
            }
        }

        let (infos, appended_infos) = self.info_tables()?;
        for region in &mut regions {
            if let Region::Content { block, info, .. } = region {
                let index = self
                    .appended
                    .iter()
                    .position(|&(start, _)| u64::from(start) == *block);
                *info = index.and_then(|index| appended_infos.get(&index).copied());
            }
        }
        for PlacedFile {
            block,
            node,
            path,
            len,
        } in file_regions
        {
            regions.push(Region::File {
                block,
                path,
                len,
                info: infos.get(&node).copied(),
            });
        }

        let tables = self.place_path_tables(&mut cursor, &mut regions)?;
        let catalog_block = self.place_catalog(&mut cursor, &mut regions)?;

        let data_end = cursor.div_ceil(SECTOR);
        regions.push(Region::Bytes {
            block: data_end,
            data: vec![0; PADDING_BLOCKS as usize * SECTOR_SIZE],
        });
        let hybrid = if self.base.system_area {
            self.opts.hybrid()
        } else {
            None
        };
        let gpt = matches!(
            hybrid.map(Hybrid::scheme),
            Some(PartitionScheme::Gpt | PartitionScheme::GptHybridMbr)
        ) || self.base.gpt_backup;
        let min = self.opts.min_image_blocks().min(1 << 32);
        let min_end = if gpt {
            (min * 4).saturating_sub(BACKUP_GPT_SECTORS) / 4
        } else {
            min
        };
        let end = (data_end + PADDING_BLOCKS).max(min_end);
        let total = if gpt {
            (end * 4 + BACKUP_GPT_SECTORS).div_ceil(4)
        } else {
            end
        };
        let volume_blocks = u32::try_from(total).map_err(|_| too_large())?;

        let descriptors = self.descriptors(volume_blocks, &tables, catalog_block)?;
        regions.push(Region::Bytes {
            block: self.base.descriptors,
            data: descriptors,
        });

        if self.base.system_area {
            let (system, tail) = self.system_area(end, total)?;
            regions.push(Region::Bytes {
                block: 0,
                data: system,
            });
            if let Some((block, data)) = tail {
                regions.push(Region::Bytes { block, data });
            }
        }

        regions.sort_by_key(Region::block);
        let report = self.report(total);
        Ok(Plan {
            regions,
            end_blocks: end,
            total_blocks: total,
            fill_gaps: self.base.fill_gaps,
            report,
        })
    }

    fn place_path_tables(
        &self,
        cursor: &mut u64,
        regions: &mut Vec<Region>,
    ) -> PlanResult<Vec<PathTableLocation>> {
        let mut tables = Vec::new();
        for ti in 0..self.trees.len() {
            let l = self.path_table(ti, false)?;
            let m = self.path_table(ti, true)?;
            let size = u32::try_from(l.len()).map_err(|_| too_large())?;
            let l_block = cursor.div_ceil(SECTOR);
            let m_block = l_block + (l.len() as u64).div_ceil(SECTOR);
            *cursor = (m_block + (m.len() as u64).div_ceil(SECTOR)) * SECTOR;
            tables.push(PathTableLocation {
                little: block_of(l_block * SECTOR)?,
                big: block_of(m_block * SECTOR)?,
                size,
            });
            regions.push(Region::Bytes {
                block: l_block,
                data: l,
            });
            regions.push(Region::Bytes {
                block: m_block,
                data: m,
            });
        }

        Ok(tables)
    }

    fn place_catalog(
        &self,
        cursor: &mut u64,
        regions: &mut Vec<Region>,
    ) -> PlanResult<Option<u32>> {
        Ok(match (self.boot_catalog()?, self.extents.get(&CATALOG)) {
            (Some(mut data), Some(extent)) => {
                let block = extent[0].0;
                data.resize(extent[0].1 as usize, 0);
                regions.push(Region::Bytes {
                    block: u64::from(block),
                    data,
                });
                Some(block)
            }
            (Some(data), None) => {
                let block = cursor.div_ceil(SECTOR);
                *cursor = block * SECTOR + data.len() as u64;
                regions.push(Region::Bytes { block, data });
                Some(block_of(block * SECTOR)?)
            }
            _ => self.base.keep_catalog,
        })
    }

    fn prepare_file_extents(&mut self, files: &[usize]) -> PlanResult<BTreeSet<usize>> {
        let mut placed = BTreeSet::new();
        for &file in files {
            let f = &self.files[file];
            let (key, len) = match f.kind {
                FileKind::Data { node, len } => (node, len),
                FileKind::Catalog { len } => (CATALOG, len),
                _ => continue,
            };
            if len == 0 || self.extents.contains_key(&key) {
                continue;
            }
            let mut extents = Vec::new();
            match self
                .contents
                .get(&key)
                .and_then(|info| info.stored.as_ref())
            {
                Some(stored) => {
                    for extent in stored {
                        extents.push((block_of(extent.offset())?, extent.len()));
                    }
                    placed.insert(key);
                }
                None => {
                    let mut remaining = len;
                    while remaining > 0 {
                        let chunk = remaining.min(MAX_EXTENT);
                        extents.push((0, chunk));
                        remaining -= chunk;
                    }
                }
            }
            self.extents.insert(key, extents);
        }

        Ok(placed)
    }

    fn place_directories(&mut self, order: &[usize], cursor: &mut u64) -> PlanResult<()> {
        for &dir in order {
            for ti in 0..self.trees.len() {
                if !self.has_dir(dir, ti) {
                    continue;
                }
                let records = self.records(dir, ti)?;
                let (start, sectors) = layout_records(*cursor, &records);
                let size = u32::try_from(sectors * SECTOR).map_err(|_| too_large())?;
                self.dir_refs
                    .insert((dir, ti), (block_of(start * SECTOR)?, size));
                *cursor = (start + sectors) * SECTOR + place_areas(&records).1;
            }
        }

        Ok(())
    }

    fn place_files(
        &mut self,
        files: &[usize],
        placed: &mut BTreeSet<usize>,
        cursor: &mut u64,
    ) -> PlanResult<Vec<PlacedFile>> {
        let mut file_regions = Vec::new();
        for &file in files {
            let f = &self.files[file];
            let (key, len) = match f.kind {
                FileKind::Data { node, len } => (node, len),
                FileKind::Catalog { len } => (CATALOG, len),
                _ => continue,
            };
            if len == 0 || !placed.insert(key) {
                continue;
            }
            let mut extents = Vec::new();
            let mut remaining = len;
            let mut at = cursor.div_ceil(SECTOR) * SECTOR;
            let first = at;
            while remaining > 0 {
                let chunk = remaining.min(MAX_EXTENT);
                extents.push((block_of(at)?, chunk));
                at += chunk;
                remaining -= chunk;
            }
            *cursor = at;
            self.extents.insert(key, extents);
            if key != CATALOG {
                file_regions.push(PlacedFile {
                    block: first / SECTOR,
                    node: key,
                    path: f.path.clone(),
                    len,
                });
            }
        }

        Ok(file_regions)
    }
}

use std::cell::Cell;

use hadris_io::{Error, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoCounts {
    pub read_calls: u64,
    pub write_calls: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub flush_calls: u64,
    pub max_read_bytes: usize,
    pub max_write_bytes: usize,
}

#[derive(Debug)]
pub struct Counted<'a, D> {
    pub inner: D,
    pub counts: &'a Cell<IoCounts>,
    pub written_blocks: Option<&'a [Cell<u64>]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteBreakdown {
    pub fat_bytes: [u64; 2],
    pub directory_bytes: u64,
    pub data_bytes: u64,
    pub fs_info_bytes: u64,
    pub other_bytes: u64,
}

impl WriteBreakdown {
    pub fn classify(image: &[u8], written_blocks: &[Cell<u64>]) -> Self {
        use hadris_fat_raw::{RootLocation, parse_boot};

        let geo = parse_boot(image[..512].try_into().unwrap()).unwrap();
        let mut directories = vec![false; written_blocks.len()];
        let mut mark = |start: u64, size: u64| {
            for block in start / 512..(start + size) / 512 {
                directories[block as usize] = true;
            }
        };
        match geo.root() {
            RootLocation::Fixed { start, size } => mark(start, size),
            RootLocation::Cluster(mut cluster) => {
                let kind = geo.kind();
                let mut visited = 0;
                loop {
                    assert!(visited < geo.max_cluster(), "cyclic root directory");
                    visited += 1;
                    mark(
                        geo.cluster_offset(cluster).unwrap(),
                        geo.cluster_size() as u64,
                    );
                    let at = (geo.fat_copy(geo.active_fat()) + kind.entry_offset(cluster as u64))
                        as usize;
                    let stored = kind.decode(cluster as u64, &image[at..at + kind.entry_len()]);
                    match kind.next(stored, geo.max_cluster()).unwrap() {
                        Some(next) => cluster = next,
                        None => break,
                    }
                }
            }
        }
        let fs_info = geo
            .fs_info_sector()
            .map(|sector| sector as u64 * geo.sector_size() as u64);
        let mut result = Self::default();
        for (index, count) in written_blocks.iter().enumerate() {
            let at = index as u64 * 512;
            let bytes = count.get() * 512;
            if let Some(copy) = (0..geo.fat_count()).find(|&copy| {
                (geo.fat_copy(copy)..geo.fat_copy(copy) + geo.fat_size()).contains(&at)
            }) {
                result.fat_bytes[copy as usize] += bytes;
            } else if directories[index] {
                result.directory_bytes += bytes;
            } else if fs_info
                .is_some_and(|start| (start..start + geo.sector_size() as u64).contains(&at))
            {
                result.fs_info_bytes += bytes;
            } else if (geo.data_start()..geo.data_end()).contains(&at) {
                result.data_bytes += bytes;
            } else {
                result.other_bytes += bytes;
            }
        }
        result
    }

    pub fn total(self) -> u64 {
        self.fat_bytes.iter().sum::<u64>()
            + self.directory_bytes
            + self.data_bytes
            + self.fs_info_bytes
            + self.other_bytes
    }
}

impl<D: ErrorType> ErrorType for Counted<'_, D> {
    type Error = D::Error;
}

impl<D: BlockDevice> BlockDevice for Counted<'_, D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn max_block_count(&self) -> u64 {
        self.inner.max_block_count()
    }

    fn disk_offset(&self) -> u64 {
        self.inner.disk_offset()
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner.read_blocks(first, buf)?;
        let mut counts = self.counts.get();
        counts.read_calls += 1;
        counts.read_bytes += buf.len() as u64;
        counts.max_read_bytes = counts.max_read_bytes.max(buf.len());
        self.counts.set(counts);
        Ok(())
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        self.inner.write_blocks(first, buf)?;
        let mut counts = self.counts.get();
        counts.write_calls += 1;
        counts.write_bytes += buf.len() as u64;
        counts.max_write_bytes = counts.max_write_bytes.max(buf.len());
        self.counts.set(counts);
        if let Some(written_blocks) = self.written_blocks {
            let start = first.get() as usize;
            for count in
                &written_blocks[start..start + buf.len() / self.block_size().get() as usize]
            {
                count.set(count.get() + 1);
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.inner.flush()?;
        let mut counts = self.counts.get();
        counts.flush_calls += 1;
        self.counts.set(counts);
        Ok(())
    }
}

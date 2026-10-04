use alloc::vec::Vec;
use hadris_fat_raw::io::ChainPos;
use hadris_fs::NodeId;

/// Bounds for optional node-driver caches, available with `alloc`.
///
/// `FatFs::mount` starts with caching disabled. `FatFs::with_cache` configures
/// a lazily populated index; no FAT or file data is read during configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOptions {
    pub(crate) positions: usize,
    pub(crate) blocks: usize,
}

impl CacheOptions {
    /// Keeps up to 32 chain positions and eight metadata device blocks.
    pub const fn new() -> Self {
        Self {
            positions: 32,
            blocks: 8,
        }
    }

    /// Sets the total chain-position bound; zero disables indexing.
    pub const fn with_chain_positions(mut self, capacity: usize) -> Self {
        self.positions = capacity;
        self
    }
    /// Sets the metadata-block bound; zero disables block caching.
    /// Payload reads bypass this cache, and writes invalidate overlapping blocks
    /// before reaching the device. No writes are deferred.
    pub const fn with_blocks(mut self, capacity: usize) -> Self {
        self.blocks = capacity;
        self
    }
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Position {
    node: NodeId,
    first: u32,
    at: ChainPos,
}

pub(crate) struct ChainCache {
    limit: usize,
    next: usize,
    entries: Vec<Position>,
}

impl ChainCache {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            next: 0,
            entries: Vec::with_capacity(limit),
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.next = 0;
    }

    pub(crate) fn best(&self, node: NodeId, first: u32, hint: ChainPos, want: u32) -> ChainPos {
        let mut best = if hint.is_known() && hint.index() <= want {
            hint
        } else {
            ChainPos::NONE
        };
        for entry in &self.entries {
            if entry.node == node
                && entry.first == first
                && entry.at.index() <= want
                && (!best.is_known() || entry.at.index() > best.index())
            {
                best = entry.at;
            }
        }
        best
    }

    pub(crate) fn insert(&mut self, node: NodeId, first: u32, at: ChainPos, stride: u32) {
        if self.limit == 0 || !at.is_known() {
            return;
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.node == node
                && entry.first == first
                && entry.at.index() / stride == at.index() / stride
        }) {
            if at.index() < entry.at.index() {
                entry.at = at;
            }
            return;
        }
        let entry = Position { node, first, at };
        if self.entries.len() < self.limit {
            self.entries.push(entry);
        } else {
            self.entries[self.next] = entry;
            self.next = (self.next + 1) % self.limit;
        }
    }
}

struct Block {
    index: u64,
    data: alloc::boxed::Box<[u8]>,
    used: u64,
}

pub(crate) struct Blocks {
    limit: usize,
    clock: u64,
    entries: Vec<Block>,
}

impl Blocks {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            clock: 0,
            entries: Vec::with_capacity(limit),
        }
    }

    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            for entry in &mut self.entries {
                entry.used = 0;
            }
            self.clock = 0;
        }
        self.clock += 1;
        self.clock
    }

    pub(crate) fn get(&mut self, index: u64) -> Option<&[u8]> {
        if self.limit == 0 {
            return None;
        }
        let used = self.tick();
        let entry = self.entries.iter_mut().find(|entry| entry.index == index)?;
        entry.used = used;
        Some(&entry.data)
    }

    pub(crate) fn insert(&mut self, index: u64, bytes: &[u8]) {
        if self.limit == 0 {
            return;
        }
        let used = self.tick();
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.index == index) {
            entry.data.copy_from_slice(bytes);
            entry.used = used;
        } else if self.entries.len() < self.limit {
            self.entries.push(Block {
                index,
                data: bytes.into(),
                used,
            });
        } else if let Some(entry) = self.entries.iter_mut().min_by_key(|entry| entry.used) {
            entry.index = index;
            entry.data.copy_from_slice(bytes);
            entry.used = used;
        }
    }

    pub(crate) fn invalidate(&mut self, first: u64, count: u64) {
        self.entries
            .retain(|entry| entry.index < first || entry.index - first >= count);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.clock = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_lru_invalidates_overlapping_ranges_and_handles_clock_wrap() {
        let mut cache = Blocks::new(2);
        cache.insert(4, &[1; 512]);
        cache.insert(5, &[2; 512]);
        assert_eq!(cache.get(4), Some([1; 512].as_slice()));
        cache.insert(6, &[3; 512]);
        assert_eq!(cache.get(5), None);
        cache.insert(4, &[9; 512]);
        cache.clock = u64::MAX;
        assert_eq!(cache.get(4), Some([9; 512].as_slice()));
        cache.invalidate(3, 2);
        assert_eq!(cache.get(4), None);
        assert_eq!(cache.get(6), Some([3; 512].as_slice()));
        cache.insert(u64::MAX, &[7; 512]);
        cache.invalidate(u64::MAX, 1);
        assert_eq!(cache.get(u64::MAX), None);
        cache.clear();
        assert!(cache.entries.is_empty());
        let mut disabled = Blocks::new(0);
        disabled.insert(0, &[0; 512]);
        assert_eq!(disabled.get(0), None);
        assert_eq!(disabled.entries.capacity(), 0);
    }

    #[test]
    fn bounded_positions_keep_guards_and_file_identity() {
        let node = NodeId::new(2).unwrap();
        let other = NodeId::new(3).unwrap();
        let mut cache = ChainCache::new(2);
        let start = ChainPos::start(5);
        let later = ChainPos::new(64, 100);
        cache.insert(node, 5, start, 64);
        cache.insert(node, 5, later, 64);
        cache.insert(node, 5, ChainPos::new(65, 101), 64);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.best(node, 5, ChainPos::NONE, 80), later);
        assert_eq!(cache.best(node, 5, later, 10), start);
        assert_eq!(cache.best(other, 5, ChainPos::NONE, 80), ChainPos::NONE);
        assert_eq!(cache.best(node, 6, ChainPos::NONE, 80), ChainPos::NONE);
        cache.insert(other, 8, ChainPos::start(8), 64);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.best(node, 5, ChainPos::NONE, 10), ChainPos::NONE);
        cache.clear();
        assert_eq!(cache.best(node, 5, ChainPos::NONE, 80), ChainPos::NONE);
        let mut disabled = ChainCache::new(0);
        disabled.insert(node, 5, start, 1);
        assert!(disabled.entries.is_empty());
        assert_eq!(disabled.entries.capacity(), 0);
    }
}

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
    pub(crate) directory_entries: usize,
    pub(crate) directory_hint: bool,
}

impl CacheOptions {
    /// Keeps up to 32 chain positions and eight metadata device blocks.
    /// Directory-prefix indexing and the listing hint remain disabled.
    pub const fn new() -> Self {
        Self {
            positions: 32,
            blocks: 8,
            directory_entries: 0,
            directory_hint: false,
        }
    }

    /// Retains one listed entry for sequential traversal, with other caches disabled.
    /// Names are copied lazily, including at most 255 UTF-16 units for a long name.
    pub const fn sequential() -> Self {
        Self {
            positions: 0,
            blocks: 0,
            directory_entries: 0,
            directory_hint: true,
        }
    }

    /// Reuses the most recently listed entry for matching lookups in that directory.
    /// Disabled by `new`; independent of the directory-prefix capacity.
    pub const fn with_directory_hint(mut self, enabled: bool) -> Self {
        self.directory_hint = enabled;
        self
    }

    /// Sets the total chain-position bound; zero disables indexing.
    pub const fn with_chain_positions(mut self, capacity: usize) -> Self {
        self.positions = capacity;
        self
    }
    /// Caches up to `capacity` parsed entries in one directory's prefix.
    /// Entries are learned during lookups, without an upfront directory scan.
    /// Switching directories replaces the prefix; overflow falls back to scanning
    /// beyond it. Creations in dense ASCII 8.3 directories also retain a sorted
    /// name set, bounded by `min(capacity, 2048)` entries, to avoid rescanning.
    /// Zero (the default) disables directory indexing and this insertion set.
    pub const fn with_directory_entries(mut self, capacity: usize) -> Self {
        self.directory_entries = capacity;
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
    bucket: u32,
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
        let bucket = at.index() / stride;
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.node == node && entry.first == first && entry.bucket == bucket)
        {
            if at.index() < entry.at.index() {
                entry.at = at;
            }
            return;
        }
        let entry = Position {
            node,
            first,
            at,
            bucket,
        };
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

    pub(crate) fn enabled(&self) -> bool {
        self.limit != 0
    }

    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            for entry in &mut self.entries {
                if entry.used != 0 {
                    entry.used = 1;
                }
            }
            self.clock = 1;
        }
        self.clock += 1;
        self.clock
    }

    pub(crate) fn get(&mut self, index: u64) -> Option<&[u8]> {
        if self.limit == 0 {
            return None;
        }
        let used = self.tick();
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.used != 0 && entry.index == index)?;
        entry.used = used;
        Some(&entry.data)
    }

    pub(crate) fn insert(&mut self, index: u64, bytes: &[u8]) {
        if self.limit == 0 {
            return;
        }
        let used = self.tick();
        let slot = self
            .entries
            .iter()
            .position(|entry| entry.used != 0 && entry.index == index)
            .or_else(|| self.entries.iter().position(|entry| entry.used == 0))
            .or_else(|| {
                if self.entries.len() == self.limit {
                    self.entries
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, entry)| entry.used)
                        .map(|(slot, _)| slot)
                } else {
                    None
                }
            });
        if let Some(slot) = slot {
            let entry = &mut self.entries[slot];
            entry.index = index;
            entry.data.copy_from_slice(bytes);
            entry.used = used;
        } else {
            self.entries.push(Block {
                index,
                data: bytes.into(),
                used,
            });
        }
    }

    pub(crate) fn invalidate(&mut self, first: u64, count: u64) {
        for entry in &mut self.entries {
            if entry.index >= first && entry.index - first < count {
                entry.used = 0;
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.used = 0;
        }
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
        assert!(cache.entries.iter().all(|entry| entry.used == 0));
        let mut disabled = Blocks::new(0);
        disabled.insert(0, &[0; 512]);
        assert_eq!(disabled.get(0), None);
        assert_eq!(disabled.entries.capacity(), 0);
    }

    #[test]
    fn block_buffers_survive_invalidation_and_clear() {
        for size in [512, 4096] {
            let mut cache = Blocks::new(2);
            cache.insert(4, &alloc::vec![1; size]);
            cache.insert(5, &alloc::vec![2; size]);
            let addresses: Vec<_> = cache
                .entries
                .iter()
                .map(|entry| entry.data.as_ptr())
                .collect();
            for round in 0..32 {
                cache.invalidate(4, 1);
                assert_eq!(cache.get(4), None);
                assert_eq!(cache.get(5), Some(alloc::vec![2; size].as_slice()));
                cache.clock = u64::MAX;
                assert_eq!(cache.get(4), None);
                cache.insert(4, &alloc::vec![round; size]);
                assert_eq!(cache.get(4), Some(alloc::vec![round; size].as_slice()));
                assert_eq!(cache.entries.len(), 2);
                for (entry, address) in cache.entries.iter().zip(&addresses) {
                    assert_eq!(entry.data.as_ptr(), *address);
                }
            }
            cache.clear();
            assert_eq!(cache.get(4), None);
            assert_eq!(cache.get(5), None);
            cache.insert(u64::MAX, &alloc::vec![3; size]);
            cache.insert(8, &alloc::vec![4; size]);
            assert_eq!(cache.get(u64::MAX), Some(alloc::vec![3; size].as_slice()));
            assert_eq!(cache.get(8), Some(alloc::vec![4; size].as_slice()));
            for (entry, address) in cache.entries.iter().zip(&addresses) {
                assert_eq!(entry.data.as_ptr(), *address);
            }
        }
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

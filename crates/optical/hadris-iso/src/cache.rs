use alloc::vec::Vec;

use crate::raw::{DirectoryRecord, SECTOR_SIZE};

/// Limits for the optional reader metadata cache.
///
/// The cache never stores file payloads. Cached images must remain unchanged;
/// call `IsoFs::clear_cache` if an external writer changes the backing device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOptions {
    pub(crate) blocks: usize,
    pub(crate) records: usize,
}
impl CacheOptions {
    /// Caches up to eight logical metadata sectors and 32 parsed records.
    pub const fn new() -> Self {
        Self {
            blocks: 8,
            records: 32,
        }
    }
    /// Sets the number of logical metadata sectors; zero disables sector caching.
    pub const fn with_blocks(mut self, capacity: usize) -> Self {
        self.blocks = capacity;
        self
    }
    /// Sets the number of parsed records; zero disables record caching.
    pub const fn with_records(mut self, capacity: usize) -> Self {
        self.records = capacity;
        self
    }
}
impl Default for CacheOptions {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug)]
struct Entry<T> {
    key: u64,
    value: T,
    used: u64,
}
#[derive(Debug)]
pub(crate) struct Entries<T> {
    limit: usize,
    clock: u64,
    entries: Vec<Entry<T>>,
}
impl<T: Copy> Entries<T> {
    fn new(limit: usize) -> Self {
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
    pub(crate) fn get(&mut self, key: u64) -> Option<T> {
        let used = self.tick();
        let entry = self.entries.iter_mut().find(|entry| entry.key == key)?;
        entry.used = used;
        Some(entry.value)
    }
    pub(crate) fn insert(&mut self, key: u64, value: T) {
        if self.limit == 0 {
            return;
        }
        let used = self.tick();
        let entry = Entry { key, value, used };
        if let Some(slot) = self.entries.iter_mut().find(|slot| slot.key == key) {
            *slot = entry;
        } else if self.entries.len() < self.limit {
            self.entries.push(entry);
        } else if let Some(slot) = self.entries.iter_mut().min_by_key(|slot| slot.used) {
            *slot = entry;
        }
    }
    fn clear(&mut self) {
        self.entries.clear();
        self.clock = 0;
    }
}
#[derive(Debug)]
pub(crate) struct ReaderCache {
    pub(crate) blocks: Entries<[u8; SECTOR_SIZE]>,
    pub(crate) records: Entries<DirectoryRecord>,
}
impl ReaderCache {
    pub(crate) fn new(options: CacheOptions) -> Self {
        Self {
            blocks: Entries::new(options.blocks),
            records: Entries::new(options.records),
        }
    }
    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.records.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::Entries;
    #[test]
    fn bounded_lru_and_clock_wrap() {
        let mut entries = Entries::new(2);
        entries.insert(1, 10);
        entries.insert(2, 20);
        assert_eq!(entries.get(1), Some(10));
        entries.insert(3, 30);
        assert_eq!(entries.get(2), None);
        assert_eq!(entries.get(1), Some(10));
        entries.insert(1, 11);
        assert_eq!(entries.get(1), Some(11));
        entries.clock = u64::MAX;
        assert_eq!(entries.get(3), Some(30));
        entries.insert(4, 40);
        assert_eq!(entries.get(1), None);
        assert_eq!(entries.get(3), Some(30));
        entries.clear();
        assert_eq!(entries.get(4), None);
        let mut zero = Entries::new(0);
        zero.insert(1, 10);
        assert_eq!(zero.get(1), None);
    }
}

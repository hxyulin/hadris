use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug)]
struct Entry {
    index: Option<u64>,
    data: Box<[u8]>,
    dirty: bool,
    used: u64,
}

#[derive(Debug)]
pub(crate) struct CacheState {
    capacity: usize,
    block_size: usize,
    entries: Vec<Entry>,
    slots: BTreeMap<u64, usize>,
    tick: u64,
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
impl CacheState {
    pub(crate) fn new(capacity: usize, block_size: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            block_size,
            entries: Vec::new(),
            slots: BTreeMap::new(),
            tick: 0,
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.entries.iter().any(|entry| entry.dirty)
    }

    pub(crate) fn lookup(&mut self, index: u64) -> Option<usize> {
        let slot = *self.slots.get(&index)?;
        self.touch(slot);
        Some(slot)
    }

    pub(crate) fn victim(&self) -> Option<usize> {
        if self.entries.len() < self.capacity {
            return None;
        }
        self.entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.used)
            .map(|(slot, _)| slot)
    }

    pub(crate) fn grow(&mut self) -> usize {
        self.entries.push(Entry {
            index: None,
            data: vec![0; self.block_size].into_boxed_slice(),
            dirty: false,
            used: 0,
        });
        self.entries.len() - 1
    }

    pub(crate) fn dirty_index(&self, slot: usize) -> Option<u64> {
        let entry = &self.entries[slot];
        if entry.dirty { entry.index } else { None }
    }

    pub(crate) fn forget(&mut self, slot: usize) {
        let entry = &mut self.entries[slot];
        if let Some(index) = entry.index.take() {
            self.slots.remove(&index);
        }
        entry.dirty = false;
    }

    pub(crate) fn assign(&mut self, slot: usize, index: u64) {
        self.entries[slot].index = Some(index);
        self.slots.insert(index, slot);
        self.touch(slot);
    }

    pub(crate) fn data(&self, slot: usize) -> &[u8] {
        &self.entries[slot].data
    }

    pub(crate) fn data_mut(&mut self, slot: usize) -> &mut [u8] {
        &mut self.entries[slot].data
    }

    pub(crate) fn mark_dirty(&mut self, slot: usize) {
        self.entries[slot].dirty = true;
    }

    pub(crate) fn clean(&mut self, slot: usize) {
        self.entries[slot].dirty = false;
    }

    pub(crate) fn next_dirty(&self) -> Option<usize> {
        self.slots
            .values()
            .copied()
            .find(|&slot| self.entries[slot].dirty)
    }

    pub(crate) fn invalidate(&mut self, first: u64, count: usize) {
        for index in first..first.saturating_add(count as u64) {
            if let Some(slot) = self.slots.get(&index).copied() {
                self.forget(slot);
            }
        }
    }

    fn touch(&mut self, slot: usize) {
        self.tick += 1;
        self.entries[slot].used = self.tick;
    }
}

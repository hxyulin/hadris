use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

const NIL: usize = usize::MAX;

/// Most blocks one coalesced flush write carries.
pub(crate) const MAX_RUN: usize = 256;

#[derive(Debug)]
struct Entry {
    index: Option<u64>,
    data: Box<[u8]>,
    prev: usize,
    next: usize,
}

/// Slots on an intrusive LRU list (head is most recent), a block-to-slot map
/// and the set of dirty block indices, so lookup, eviction and flush never
/// scan every slot.
#[derive(Debug)]
pub(crate) struct CacheState {
    capacity: usize,
    block_size: usize,
    entries: Vec<Entry>,
    slots: BTreeMap<u64, usize>,
    dirty: BTreeSet<u64>,
    head: usize,
    tail: usize,
    run: Vec<u8>,
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
impl CacheState {
    pub(crate) fn new(capacity: usize, block_size: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            block_size,
            entries: Vec::new(),
            slots: BTreeMap::new(),
            dirty: BTreeSet::new(),
            head: NIL,
            tail: NIL,
            run: Vec::new(),
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    pub(crate) fn lookup(&mut self, index: u64) -> Option<usize> {
        let slot = *self.slots.get(&index)?;
        self.touch(slot);
        Some(slot)
    }

    pub(crate) fn peek(&self, index: u64) -> Option<usize> {
        self.slots.get(&index).copied()
    }

    /// The least recently used slot once the cache is full.
    pub(crate) fn victim(&self) -> Option<usize> {
        if self.entries.len() < self.capacity {
            None
        } else {
            Some(self.tail)
        }
    }

    pub(crate) fn grow(&mut self) -> usize {
        let slot = self.entries.len();
        self.entries.push(Entry {
            index: None,
            data: vec![0; self.block_size].into_boxed_slice(),
            prev: NIL,
            next: NIL,
        });
        self.push_front(slot);
        slot
    }

    pub(crate) fn dirty_index(&self, slot: usize) -> Option<u64> {
        self.entries[slot]
            .index
            .filter(|index| self.dirty.contains(index))
    }

    /// Drops the block a slot holds and makes the slot the next victim.
    pub(crate) fn forget(&mut self, slot: usize) {
        if let Some(index) = self.entries[slot].index.take() {
            self.slots.remove(&index);
            self.dirty.remove(&index);
        }
        self.unlink(slot);
        self.push_back(slot);
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
        if let Some(index) = self.entries[slot].index {
            self.dirty.insert(index);
        }
    }

    pub(crate) fn first_dirty(&self) -> Option<u64> {
        self.dirty.first().copied()
    }

    /// How many consecutive dirty blocks start at `start`, at most [`MAX_RUN`].
    pub(crate) fn dirty_run(&self, start: u64) -> usize {
        let mut len = 1;
        while len < MAX_RUN && self.dirty.contains(&(start + len as u64)) {
            len += 1;
        }
        len
    }

    /// Copies `len` cached blocks from `start` into one buffer for a single
    /// device write. Every block in the range must be cached.
    pub(crate) fn gather(&mut self, start: u64, len: usize) -> &[u8] {
        self.run.clear();
        for index in start..start + len as u64 {
            let slot = self.slots[&index];
            self.run.extend_from_slice(&self.entries[slot].data);
        }
        &self.run
    }

    pub(crate) fn clean_range(&mut self, start: u64, len: usize) {
        for index in start..start + len as u64 {
            self.dirty.remove(&index);
        }
    }

    /// Dirty blocks in `first..first + count`.
    pub(crate) fn dirty_in(&self, first: u64, count: usize) -> impl Iterator<Item = u64> + '_ {
        self.dirty
            .range(first..first.saturating_add(count as u64))
            .copied()
    }

    /// Cached blocks in `first..first + count` with their slots.
    pub(crate) fn cached_in(
        &self,
        first: u64,
        count: usize,
    ) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.slots
            .range(first..first.saturating_add(count as u64))
            .map(|(&index, &slot)| (index, slot))
    }

    pub(crate) fn invalidate(&mut self, first: u64, count: usize) {
        let slots: Vec<usize> = self.cached_in(first, count).map(|(_, slot)| slot).collect();
        for slot in slots {
            self.forget(slot);
        }
    }

    fn touch(&mut self, slot: usize) {
        if self.head != slot {
            self.unlink(slot);
            self.push_front(slot);
        }
    }

    fn unlink(&mut self, slot: usize) {
        let (prev, next) = (self.entries[slot].prev, self.entries[slot].next);
        if prev == NIL {
            self.head = next
        } else {
            self.entries[prev].next = next
        }
        if next == NIL {
            self.tail = prev
        } else {
            self.entries[next].prev = prev
        }
        self.entries[slot].prev = NIL;
        self.entries[slot].next = NIL;
    }

    fn push_front(&mut self, slot: usize) {
        self.entries[slot].next = self.head;
        if self.head == NIL {
            self.tail = slot
        } else {
            self.entries[self.head].prev = slot
        }
        self.head = slot;
    }

    fn push_back(&mut self, slot: usize) {
        self.entries[slot].prev = self.tail;
        if self.tail == NIL {
            self.head = slot
        } else {
            self.entries[self.tail].next = slot
        }
        self.tail = slot;
    }
}

use crate::device::check_blocks;
use crate::{BlockIndex, BlockSize};
use alloc::vec::Vec;
use core::convert::Infallible;

#[derive(Debug, Default)]
struct Window {
    first: u64,
    count: usize,
    data: Vec<u8>,
}

/// The two read-ahead windows and the planning both modes share.
#[derive(Debug)]
pub(crate) struct Windows {
    capacity: usize,
    block_size: usize,
    windows: [Window; 2],
    recent: usize,
}

/// What a read does after [`Windows::plan`].
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) enum Plan {
    /// The windows held the blocks, and they are in the buffer.
    Hit,
    /// The read goes straight to the device.
    Bypass,
    /// The read fills `fetched` blocks of window `slot`, then
    /// [`Windows::fill`] copies the request out.
    Fetch { slot: usize, fetched: usize },
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
impl Windows {
    pub(crate) fn new(block_size: usize, capacity: usize, block_count: u64) -> Self {
        let blocks = usize::try_from(block_count).unwrap_or(usize::MAX);
        Self {
            capacity: capacity.min(isize::MAX as usize / block_size).min(blocks),
            block_size,
            windows: Default::default(),
            recent: 1,
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn clear(&mut self) {
        for window in &mut self.windows {
            window.count = 0;
        }
    }

    /// Serves a read of `buf` at block `first` from the windows, or plans
    /// the device read that serves it. `size` and `block_count` are the
    /// device's now; a request outside the device bypasses the windows.
    pub(crate) fn plan(
        &mut self,
        block_size: BlockSize,
        block_count: u64,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Plan {
        let size = block_size.get() as usize;
        if size != self.block_size {
            self.windows = Default::default();
            self.block_size = size;
            self.capacity = self.capacity.min(isize::MAX as usize / size);
        }
        let in_range =
            check_blocks::<Infallible>(block_size, block_count, first, buf.len()).is_ok();
        if self.capacity == 0 || buf.is_empty() || !in_range {
            return Plan::Bypass;
        }
        let first = first.get();
        let count = buf.len() / size;
        for (slot, window) in self.windows.iter().enumerate() {
            if first >= window.first {
                let offset = first - window.first;
                if offset <= window.count as u64 && count as u64 <= window.count as u64 - offset {
                    let at = offset as usize * size;
                    buf.copy_from_slice(&window.data[at..at + buf.len()]);
                    self.recent = slot;
                    return Plan::Hit;
                }
            }
        }
        let adjacent = self
            .windows
            .iter()
            .position(|w| w.count != 0 && w.first + w.count as u64 == first);
        let slot = adjacent.unwrap_or(if self.capacity == 1 {
            0
        } else {
            1 - self.recent
        });
        let capacity = if slot == 0 {
            self.capacity - self.capacity / 2
        } else {
            self.capacity / 2
        };
        if count > capacity {
            return Plan::Bypass;
        }
        let available = usize::try_from(block_count - first).unwrap_or(usize::MAX);
        let fetched = if adjacent.is_some() {
            capacity.min(available)
        } else {
            count
        };
        let window = &mut self.windows[slot];
        window.count = 0;
        window.data.resize(capacity * size, 0);
        Plan::Fetch { slot, fetched }
    }

    /// The buffer a [`Plan::Fetch`] reads into.
    pub(crate) fn target(&mut self, slot: usize, fetched: usize) -> &mut [u8] {
        &mut self.windows[slot].data[..fetched * self.block_size]
    }

    /// Records a completed [`Plan::Fetch`] and copies the request into `buf`.
    pub(crate) fn fill(&mut self, slot: usize, fetched: usize, first: u64, buf: &mut [u8]) {
        let window = &mut self.windows[slot];
        window.first = first;
        window.count = fetched;
        buf.copy_from_slice(&window.data[..buf.len()]);
        self.recent = slot;
    }

    /// Drops the windows a write of `len` bytes at block `first` overlaps.
    pub(crate) fn invalidate(&mut self, size: usize, first: u64, len: usize) {
        let end = first.saturating_add(len.div_ceil(size) as u64);
        for window in &mut self.windows {
            if window.first < end && first < window.first + window.count as u64 {
                window.count = 0;
            }
        }
    }
}

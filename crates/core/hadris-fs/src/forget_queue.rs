//! `Volume`'s deferred forgets, without allocation.
//!
//! A handle dropped while another call holds the driver lock cannot forget
//! its node directly, so the node waits here until the next lock drains it.
//! Repeated forgets of one node share an entry with a count, so the queue
//! only fills with `CAPACITY` distinct nodes. When it is full, `Volume`
//! waits for the driver lock instead of dropping the forget.

use crate::NodeId;

/// Distinct nodes the queue holds.
pub(crate) const CAPACITY: usize = 16;

/// Pending forgets: node id and how many times it was forgotten.
#[derive(Debug)]
pub(crate) struct ForgetQueue {
    entries: [(u64, u32); CAPACITY],
    len: usize,
}

impl ForgetQueue {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [(0, 0); CAPACITY],
            len: 0,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Queues one forget of `node`. Returns `false`, queueing nothing, when
    /// `node` is new and the queue is full.
    pub(crate) fn push(&mut self, node: NodeId) -> bool {
        let id = node.get();
        if let Some(entry) = self.entries[..self.len].iter_mut().find(|e| e.0 == id) {
            entry.1 += 1;
            return true;
        }
        if self.len == CAPACITY {
            return false;
        }
        self.entries[self.len] = (id, 1);
        self.len += 1;
        true
    }

    /// Empties the queue, calling `forget` once per queued forget.
    pub(crate) fn drain(&mut self, mut forget: impl FnMut(NodeId)) {
        for &(id, count) in &self.entries[..self.len] {
            for _ in 0..count {
                forget(NodeId::new(id));
            }
        }
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{CAPACITY, ForgetQueue};
    use crate::NodeId;

    #[test]
    fn coalesces_and_refuses_when_full() {
        let mut queue = ForgetQueue::new();
        for id in 0..CAPACITY as u64 {
            assert!(queue.push(NodeId::new(id + 10)));
        }
        assert!(queue.push(NodeId::new(10)));
        assert!(!queue.push(NodeId::new(999)));
        let mut seen = [0u32; CAPACITY];
        queue.drain(|node| seen[(node.get() - 10) as usize] += 1);
        assert_eq!(seen[0], 2);
        assert!(seen[1..].iter().all(|&n| n == 1));
        assert!(queue.is_empty());
        assert!(queue.push(NodeId::new(999)));
    }
}

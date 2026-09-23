//! `Volume`'s deferred forgets and closes, without allocation.
//!
//! A handle dropped while another call holds the driver lock cannot close or
//! forget its node directly, so the node waits here until the next lock
//! drains it. Repeated calls on one node share an entry with counts, so the
//! queue only fills with `CAPACITY` distinct nodes. When it is full,
//! `Volume` waits for the driver lock instead of dropping the call.

use crate::NodeId;

/// Distinct nodes the queue holds.
pub(crate) const CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u64,
    closes: u32,
    forgets: u32,
}

/// Pending `close_node` and `forget` calls per node.
#[derive(Debug)]
pub(crate) struct ForgetQueue {
    entries: [Entry; CAPACITY],
    len: usize,
}

impl ForgetQueue {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [Entry {
                id: 0,
                closes: 0,
                forgets: 0,
            }; CAPACITY],
            len: 0,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn entry(&mut self, node: NodeId) -> Option<&mut Entry> {
        let id = node.get();
        if let Some(at) = self.entries[..self.len].iter().position(|e| e.id == id) {
            return Some(&mut self.entries[at]);
        }
        if self.len == CAPACITY {
            return None;
        }
        self.entries[self.len] = Entry {
            id,
            closes: 0,
            forgets: 0,
        };
        self.len += 1;
        Some(&mut self.entries[self.len - 1])
    }

    /// Queues one forget of `node`. Returns `false`, queueing nothing, when
    /// `node` is new and the queue is full.
    pub(crate) fn push(&mut self, node: NodeId) -> bool {
        self.entry(node).map(|e| e.forgets += 1).is_some()
    }

    /// Queues one close of `node`, as [`push`](Self::push) does a forget.
    pub(crate) fn push_close(&mut self, node: NodeId) -> bool {
        self.entry(node).map(|e| e.closes += 1).is_some()
    }

    /// Empties the queue. Each node's closes run before its forgets, since
    /// a handle closes its node before it forgets it.
    pub(crate) fn drain(&mut self, mut close: impl FnMut(NodeId), mut forget: impl FnMut(NodeId)) {
        for entry in &self.entries[..self.len] {
            for _ in 0..entry.closes {
                close(NodeId::new(entry.id));
            }
            for _ in 0..entry.forgets {
                forget(NodeId::new(entry.id));
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
        assert!(!queue.push_close(NodeId::new(999)));
        let mut seen = [0u32; CAPACITY];
        queue.drain(|_| {}, |node| seen[(node.get() - 10) as usize] += 1);
        assert_eq!(seen[0], 2);
        assert!(seen[1..].iter().all(|&n| n == 1));
        assert!(queue.is_empty());
        assert!(queue.push(NodeId::new(999)));
    }

    #[test]
    fn closes_run_before_forgets() {
        let mut queue = ForgetQueue::new();
        assert!(queue.push(NodeId::new(5)));
        assert!(queue.push_close(NodeId::new(5)));
        let order = core::cell::RefCell::new(std::vec::Vec::new());
        queue.drain(
            |n| order.borrow_mut().push(("close", n.get())),
            |n| order.borrow_mut().push(("forget", n.get())),
        );
        assert_eq!(order.into_inner(), [("close", 5), ("forget", 5)]);
    }
}

//! `Volume`'s deferred calls.
//!
//! A handle dropped while another call holds the driver lock cannot close or
//! forget its node directly, so the node waits here until the next lock
//! drains it. Repeated calls on one node share an entry with counts. With
//! `alloc` the queue grows as needed; without it, it holds `CAPACITY`
//! distinct nodes and refuses more.

use crate::NodeId;

/// Distinct nodes the queue holds without `alloc`.
#[cfg_attr(feature = "alloc", allow(dead_code))]
pub(crate) const CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u64,
    publishes: u32,
    closes: u32,
    forgets: u32,
}

impl Entry {
    const fn new(id: u64) -> Self {
        Self {
            id,
            publishes: 0,
            closes: 0,
            forgets: 0,
        }
    }
}

/// A queued call on a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Publish,
    Close,
    Forget,
}

/// Pending `publish_node`, `close_node` and `forget` calls per node.
#[derive(Debug)]
pub(crate) struct ForgetQueue {
    #[cfg(feature = "alloc")]
    entries: alloc::vec::Vec<Entry>,
    #[cfg(not(feature = "alloc"))]
    entries: [Entry; CAPACITY],
    #[cfg(not(feature = "alloc"))]
    len: usize,
}

impl ForgetQueue {
    pub(crate) const fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            entries: alloc::vec::Vec::new(),
            #[cfg(not(feature = "alloc"))]
            entries: [Entry::new(0); CAPACITY],
            #[cfg(not(feature = "alloc"))]
            len: 0,
        }
    }

    fn queued(&mut self) -> &mut [Entry] {
        #[cfg(feature = "alloc")]
        return &mut self.entries;
        #[cfg(not(feature = "alloc"))]
        return &mut self.entries[..self.len];
    }

    fn entry(&mut self, node: NodeId) -> Option<&mut Entry> {
        let id = node.get();
        if let Some(at) = self.queued().iter().position(|e| e.id == id) {
            return Some(&mut self.queued()[at]);
        }
        #[cfg(feature = "alloc")]
        {
            self.entries.push(Entry::new(id));
            self.entries.last_mut()
        }
        #[cfg(not(feature = "alloc"))]
        {
            if self.len == CAPACITY {
                return None;
            }
            self.entries[self.len] = Entry::new(id);
            self.len += 1;
            Some(&mut self.entries[self.len - 1])
        }
    }

    /// Queues one forget of `node`. Returns `false`, queueing nothing, when
    /// `node` is new and the queue is full, which needs a build without
    /// `alloc`.
    pub(crate) fn push(&mut self, node: NodeId) -> bool {
        self.entry(node).map(|e| e.forgets += 1).is_some()
    }

    /// Queues one close of `node`, as [`push`](Self::push) does a forget.
    pub(crate) fn push_close(&mut self, node: NodeId) -> bool {
        self.entry(node).map(|e| e.closes += 1).is_some()
    }

    /// Queues one publish of `node` if a close of it is queued, and returns
    /// whether it did. Never needs a new entry.
    #[cfg_attr(not(feature = "sync"), allow(dead_code))]
    pub(crate) fn push_publish(&mut self, node: NodeId) -> bool {
        let id = node.get();
        match self
            .queued()
            .iter_mut()
            .find(|e| e.id == id && e.closes > 0)
        {
            Some(entry) => {
                entry.publishes += 1;
                true
            }
            None => false,
        }
    }

    /// Moves the queued calls out, leaving the queue empty, so they can run
    /// without holding the queue's lock.
    pub(crate) fn take(&mut self) -> Self {
        core::mem::replace(self, Self::new())
    }

    /// Empties the queue. Each node's publishes run before its closes and
    /// its closes before its forgets, the order a handle makes them in.
    pub(crate) fn drain(&mut self, mut call: impl FnMut(Op, NodeId)) {
        for entry in self.queued().iter() {
            let node = NodeId::new(entry.id);
            for _ in 0..entry.publishes {
                call(Op::Publish, node);
            }
            for _ in 0..entry.closes {
                call(Op::Close, node);
            }
            for _ in 0..entry.forgets {
                call(Op::Forget, node);
            }
        }
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::{CAPACITY, ForgetQueue, Op};
    use crate::NodeId;

    #[test]
    fn coalesces_entries() {
        let mut queue = ForgetQueue::new();
        for id in 0..CAPACITY as u64 {
            assert!(queue.push(NodeId::new(id + 10)));
        }
        assert!(queue.push(NodeId::new(10)));
        let mut seen = [0u32; CAPACITY];
        queue.drain(|_, node| seen[(node.get() - 10) as usize] += 1);
        assert_eq!(seen[0], 2);
        assert!(seen[1..].iter().all(|&n| n == 1));
        assert!(queue.take().queued().is_empty());
    }

    #[cfg(not(feature = "alloc"))]
    #[test]
    fn refuses_a_new_node_when_full() {
        let mut queue = ForgetQueue::new();
        for id in 0..CAPACITY as u64 {
            assert!(queue.push(NodeId::new(id + 10)));
        }
        assert!(queue.push(NodeId::new(10)));
        assert!(!queue.push(NodeId::new(999)));
        assert!(!queue.push_close(NodeId::new(999)));
        queue.drain(|_, _| {});
        assert!(queue.push(NodeId::new(999)));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn grows_past_the_fixed_capacity() {
        let mut queue = ForgetQueue::new();
        for id in 0..4 * CAPACITY as u64 {
            assert!(queue.push_close(NodeId::new(id + 1)));
        }
        let mut closes = 0;
        queue.drain(|op, _| closes += (op == Op::Close) as usize);
        assert_eq!(closes, 4 * CAPACITY);
    }

    #[test]
    fn publishes_join_a_queued_close_and_run_first() {
        let mut queue = ForgetQueue::new();
        assert!(!queue.push_publish(NodeId::new(5)));
        assert!(queue.push(NodeId::new(5)));
        assert!(!queue.push_publish(NodeId::new(5)));
        assert!(queue.push_close(NodeId::new(5)));
        assert!(queue.push_publish(NodeId::new(5)));
        let mut order = std::vec::Vec::new();
        queue.take().drain(|op, n| order.push((op, n.get())));
        assert_eq!(order, [(Op::Publish, 5), (Op::Close, 5), (Op::Forget, 5)]);
    }
}

#[cfg(all(test, feature = "sync", not(feature = "alloc")))]
mod volume_tests {
    use super::CAPACITY;
    use crate::sync::{FileSystem, FsDriver, Volume};
    use crate::{
        Capabilities, DirCursor, DirEntry, ErrorKind, FsResult, FsStats, Metadata, Name, NameBuf,
        NodeId,
    };

    #[derive(Debug)]
    struct Never;

    impl core::fmt::Display for Never {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("never")
        }
    }

    impl core::error::Error for Never {}

    struct Counting(u32);

    impl FsDriver for Counting {
        type DeviceError = Never;
        fn capabilities(&self) -> Capabilities {
            Capabilities::new()
        }
        fn root(&self) -> NodeId {
            NodeId::new(1)
        }
        fn lookup(&mut self, _: NodeId, _: &Name) -> FsResult<NodeId, Never> {
            Err(ErrorKind::Unsupported.into())
        }
        fn node_metadata(&mut self, _: NodeId) -> FsResult<Metadata, Never> {
            Err(ErrorKind::Unsupported.into())
        }
        fn read_dir_entry(
            &mut self,
            _: NodeId,
            _: &mut DirCursor,
            _: &mut NameBuf,
        ) -> FsResult<Option<DirEntry>, Never> {
            Ok(None)
        }
        fn read_at(&mut self, _: NodeId, _: u64, _: &mut [u8]) -> FsResult<usize, Never> {
            Ok(0)
        }
        fn stats(&mut self) -> FsResult<FsStats, Never> {
            Err(ErrorKind::Unsupported.into())
        }
        fn forget(&mut self, _: NodeId) {
            self.0 += 1;
        }
    }

    #[test]
    fn a_full_queue_under_the_lock_panics_instead_of_waiting() {
        let vol = Volume::local(Counting(0));
        let guard = vol.lock();
        for id in 0..CAPACITY as u64 {
            vol.forget(NodeId::new(id + 10));
        }
        let full = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            vol.forget(NodeId::new(999));
        }));
        assert!(full.is_err());
        drop(guard);
        assert_eq!(vol.into_inner().0, CAPACITY as u32);
    }
}

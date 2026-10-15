use alloc::collections::BTreeMap;

use hadris_fs::{ErrorKind, Name, NodeId, OpenMode};

/// `NodeId::new` for ids that are not 0 by construction.
pub(crate) const fn node_id(raw: u64) -> NodeId {
    match NodeId::new(raw) {
        Some(id) => id,
        None => RESERVED,
    }
}

/// The id of the root directory.
pub(crate) const ROOT: NodeId = node_id(1);
/// The id of the table slot `create` reserves before it writes anything.
/// Never handed out.
pub(crate) const RESERVED: NodeId = match NodeId::new(1 << 63) {
    Some(id) => id,
    None => panic!("not 0"),
};

/// The name a query must be, as a string: not `.` or `..`.
pub(crate) fn entry_name(name: &Name, invalid: ErrorKind) -> Result<&str, ErrorKind> {
    match name.to_str() {
        Ok("." | "..") => Err(ErrorKind::InvalidInput),
        Ok(text) => Ok(text),
        Err(_) => Err(invalid),
    }
}

/// The state of a pinned node that the pin, dirty and open accounting of
/// [`Table`] reads and changes.
pub(crate) trait NodeState {
    fn is_dir(&self) -> bool;
    /// The entry lacks the node's sizes and modification time. A dirty node
    /// holds one pin of the driver's own until it is written.
    fn is_dirty(&self) -> bool;
    fn set_dirty(&mut self, dirty: bool);
    /// Opens not yet closed.
    fn opens(&self) -> u32;
    fn set_opens(&mut self, opens: u32);
    /// Removed while pinned: every method but `forget` and `close` answers
    /// `NotFound`.
    fn is_unlinked(&self) -> bool;
    /// Marks the node removed: its entry and clusters are gone.
    fn unlink(&mut self);
}

/// Per-node driver state with a pin count per node, for pinned and open
/// nodes. An entry goes when its last pin does, or on `remove`.
pub(crate) struct Table<V> {
    nodes: BTreeMap<NodeId, (u32, V)>,
    limit: Option<usize>,
}

impl<V> Table<V> {
    /// An empty table holding at most `limit` entries.
    pub(crate) const fn new(limit: Option<usize>) -> Self {
        Self {
            nodes: BTreeMap::new(),
            limit,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(crate) fn get(&self, id: NodeId) -> Option<&V> {
        self.nodes.get(&id).map(|(_, value)| value)
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> Option<&mut V> {
        self.nodes.get_mut(&id).map(|(_, value)| value)
    }

    /// The pin count of `id`, 0 when absent.
    pub(crate) fn pins(&self, id: NodeId) -> u32 {
        self.nodes.get(&id).map_or(0, |(pins, _)| *pins)
    }

    /// The first entry for which `pred` holds.
    pub(crate) fn find(&self, mut pred: impl FnMut(NodeId, &V) -> bool) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(id, (_, value))| pred(**id, value))
            .map(|(id, _)| *id)
    }

    /// The first entry with an id in `from..=to` for which `pred` holds.
    pub(crate) fn find_in(
        &self,
        from: NodeId,
        to: NodeId,
        mut pred: impl FnMut(NodeId, &V) -> bool,
    ) -> Option<NodeId> {
        if from > to {
            return None;
        }
        self.nodes
            .range(from..=to)
            .find(|(id, (_, value))| pred(**id, value))
            .map(|(id, _)| *id)
    }

    pub(crate) fn for_each_mut(&mut self, mut f: impl FnMut(NodeId, &mut V)) {
        for (id, (_, value)) in &mut self.nodes {
            f(*id, value);
        }
    }

    /// Adds `id` with one pin, or replaces its state and keeps its pins.
    /// [`ErrorKind::LimitExceeded`] when a new entry would pass the limit.
    pub(crate) fn insert(&mut self, id: NodeId, value: V) -> Result<(), ErrorKind> {
        if let Some((_, old)) = self.nodes.get_mut(&id) {
            *old = value;
            return Ok(());
        }
        if self.limit.is_some_and(|limit| self.nodes.len() >= limit) {
            return Err(ErrorKind::LimitExceeded);
        }
        self.nodes.insert(id, (1, value));
        Ok(())
    }

    /// Adds a pin and returns the new count, or `None` when absent.
    pub(crate) fn pin(&mut self, id: NodeId) -> Option<u32> {
        let (pins, _) = self.nodes.get_mut(&id)?;
        *pins = pins.saturating_add(1);
        Some(*pins)
    }

    /// Drops a pin and returns the remaining count, or `None` when absent.
    /// The entry goes at zero.
    pub(crate) fn unpin(&mut self, id: NodeId) -> Option<u32> {
        let (pins, _) = self.nodes.get_mut(&id)?;
        *pins = pins.saturating_sub(1);
        let pins = *pins;
        if pins == 0 {
            self.nodes.remove(&id);
        }
        Some(pins)
    }

    pub(crate) fn remove(&mut self, id: NodeId) -> Option<V> {
        self.nodes.remove(&id).map(|(_, value)| value)
    }
}

impl<V: NodeState> Table<V> {
    /// Pins held by callers, not counting the driver's own pin on a dirty
    /// node.
    pub(crate) fn user_pins(&self, id: NodeId) -> u32 {
        let dirty = self.get(id).is_some_and(V::is_dirty);
        self.pins(id).saturating_sub(dirty as u32)
    }

    pub(crate) fn is_open(&self, id: NodeId) -> bool {
        self.get(id).is_some_and(|node| node.opens() > 0)
    }

    /// Marks a node dirty, taking the driver's pin on it the first time.
    pub(crate) fn mark_dirty(&mut self, id: NodeId) {
        let fresh = match self.get_mut(id) {
            Some(node) if !node.is_dirty() => {
                node.set_dirty(true);
                true
            }
            _ => false,
        };
        if fresh {
            self.pin(id);
        }
    }

    /// Marks a node written, dropping the driver's pin on a dirty one.
    pub(crate) fn clean(&mut self, id: NodeId) {
        let was_dirty = match self.get_mut(id) {
            Some(node) if node.is_dirty() => {
                node.set_dirty(false);
                true
            }
            _ => false,
        };
        if was_dirty {
            self.unpin(id);
        }
    }

    /// Drops the state of a node whose entry was just removed. A node
    /// callers still pin stays, marked unlinked, so its id answers
    /// `NotFound` until the last `forget`; returns whether it stayed.
    pub(crate) fn mark_unlinked(&mut self, id: NodeId) -> bool {
        self.clean(id);
        if self.pins(id) == 0 {
            self.remove(id);
            return false;
        }
        match self.get_mut(id) {
            Some(node) => {
                node.unlink();
                true
            }
            None => false,
        }
    }

    /// Counts one `open` of a pinned file, refusing the root, directories,
    /// removed nodes, writes to a read-only volume and ids that are not
    /// pinned.
    pub(crate) fn open(
        &mut self,
        id: NodeId,
        mode: OpenMode,
        read_only: bool,
    ) -> Result<(), ErrorKind> {
        if id == ROOT {
            return Err(ErrorKind::IsADirectory);
        }
        match self.get_mut(id) {
            Some(node) if node.is_unlinked() => Err(ErrorKind::NotFound),
            Some(node) if node.is_dir() => Err(ErrorKind::IsADirectory),
            Some(_) if mode == OpenMode::Write && read_only => Err(ErrorKind::ReadOnly),
            Some(node) => {
                node.set_opens(node.opens().saturating_add(1));
                Ok(())
            }
            None => Err(ErrorKind::InvalidHandle),
        }
    }

    /// Ends one `open`.
    pub(crate) fn close(&mut self, id: NodeId) {
        if let Some(node) = self.get_mut(id) {
            node.set_opens(node.opens().saturating_sub(1));
        }
    }

    /// Drops `count` caller pins of a node; the root is ignored. A removed
    /// node leaves the table with its last pin.
    pub(crate) fn forget(&mut self, id: NodeId, count: u64) {
        if id == ROOT {
            return;
        }
        for _ in 0..count.min(u64::from(self.user_pins(id))) {
            if self.unpin(id) == Some(0) && self.get(id).is_some_and(V::is_unlinked) {
                self.remove(id);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> NodeId {
        NodeId::new(raw).unwrap()
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct State {
        dir: bool,
        dirty: bool,
        opens: u32,
        unlinked: bool,
        entry: u64,
    }

    impl NodeState for State {
        fn is_dir(&self) -> bool {
            self.dir
        }
        fn is_dirty(&self) -> bool {
            self.dirty
        }
        fn set_dirty(&mut self, dirty: bool) {
            self.dirty = dirty;
        }
        fn opens(&self) -> u32 {
            self.opens
        }
        fn set_opens(&mut self, opens: u32) {
            self.opens = opens;
        }
        fn is_unlinked(&self) -> bool {
            self.unlinked
        }
        fn unlink(&mut self) {
            self.unlinked = true;
            self.entry = u64::MAX;
        }
    }

    #[test]
    fn dirty_nodes_hold_one_driver_pin() {
        let mut table = Table::new(None);
        table.insert(id(2), State::default()).unwrap();
        table.mark_dirty(id(2));
        table.mark_dirty(id(2));
        assert_eq!(table.pins(id(2)), 2);
        assert_eq!(table.user_pins(id(2)), 1);
        table.forget(id(2), 5);
        assert_eq!(table.pins(id(2)), 1);
        assert!(table.get(id(2)).is_some_and(|node| node.dirty));
        table.clean(id(2));
        table.clean(id(2));
        assert_eq!(table.get(id(2)), None);
    }

    #[test]
    fn opens_refuse_what_cannot_be_opened() {
        let mut table = Table::new(None);
        table.insert(id(2), State::default()).unwrap();
        table
            .insert(
                id(3),
                State {
                    dir: true,
                    ..State::default()
                },
            )
            .unwrap();
        assert_eq!(
            table.open(ROOT, OpenMode::Read, false),
            Err(ErrorKind::IsADirectory)
        );
        assert_eq!(
            table.open(id(3), OpenMode::Read, false),
            Err(ErrorKind::IsADirectory)
        );
        assert_eq!(
            table.open(id(9), OpenMode::Read, false),
            Err(ErrorKind::InvalidHandle)
        );
        assert_eq!(
            table.open(id(2), OpenMode::Write, true),
            Err(ErrorKind::ReadOnly)
        );
        assert_eq!(table.open(id(2), OpenMode::Read, true), Ok(()));
        assert_eq!(table.open(id(2), OpenMode::Write, false), Ok(()));
        assert!(table.is_open(id(2)));
        table.close(id(2));
        assert!(table.is_open(id(2)));
        table.close(id(2));
        table.close(id(2));
        assert!(!table.is_open(id(2)));
        assert_eq!(table.get(id(2)).map(|node| node.opens), Some(0));
    }

    #[test]
    fn unlinked_nodes_stay_until_their_last_pin() {
        let mut table = Table::new(None);
        table
            .insert(
                id(2),
                State {
                    entry: 64,
                    ..State::default()
                },
            )
            .unwrap();
        table.insert(id(3), State::default()).unwrap();
        table.pin(id(2));
        table.mark_dirty(id(2));
        assert!(table.mark_unlinked(id(2)));
        let node = *table.get(id(2)).unwrap();
        assert!(node.unlinked && !node.dirty);
        assert_eq!(node.entry, u64::MAX);
        assert_eq!(
            table.open(id(2), OpenMode::Read, false),
            Err(ErrorKind::NotFound)
        );
        table.forget(id(2), 1);
        assert_eq!(table.pins(id(2)), 1);
        table.forget(id(2), 1);
        assert_eq!(table.get(id(2)), None);
        table.forget(id(3), 1);
        assert!(!table.mark_unlinked(id(3)));
        assert!(table.is_empty());
        table.forget(ROOT, 1);
    }

    #[test]
    fn entry_names_refuse_dot_entries() {
        assert_eq!(
            entry_name(Name::new("a.txt"), ErrorKind::NotFound),
            Ok("a.txt")
        );
        assert_eq!(
            entry_name(Name::new("."), ErrorKind::NotFound),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            entry_name(Name::new(".."), ErrorKind::NotFound),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            entry_name(Name::new(b"\xff"), ErrorKind::NotFound),
            Err(ErrorKind::NotFound)
        );
        assert_ne!(ROOT, RESERVED);
        assert_eq!(node_id(0), RESERVED);
    }

    #[test]
    fn pins_and_removal() {
        let mut table = Table::new(None);
        assert!(table.is_empty());
        assert_eq!(table.insert(id(1), 10), Ok(()));
        assert_eq!(table.pins(id(1)), 1);
        assert_eq!(table.pin(id(1)), Some(2));
        assert_eq!(table.pin(id(9)), None);
        assert_eq!(table.unpin(id(1)), Some(1));
        assert_eq!(table.get(id(1)), Some(&10));
        assert_eq!(table.unpin(id(1)), Some(0));
        assert_eq!(table.get(id(1)), None);
        assert_eq!(table.unpin(id(1)), None);
        table.insert(id(2), 20).unwrap();
        table.insert(id(3), 30).unwrap();
        table.pin(id(3));
        table.insert(id(3), 31).unwrap();
        assert_eq!(table.pins(id(3)), 2);
        *table.get_mut(id(2)).unwrap() += 1;
        assert_eq!(table.find(|_, value| *value == 21), Some(id(2)));
        let mut sum = 0;
        table.for_each_mut(|_, value| {
            *value += 1;
            sum += *value;
        });
        assert_eq!(sum, 22 + 32);
        assert_eq!(table.remove(id(3)), Some(32));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn the_limit_caps_new_entries() {
        let mut table = Table::new(Some(1));
        table.insert(id(1), 1).unwrap();
        assert_eq!(table.insert(id(2), 2), Err(ErrorKind::LimitExceeded));
        assert_eq!(table.insert(id(1), 3), Ok(()));
        table.remove(id(1));
        assert_eq!(table.insert(id(2), 2), Ok(()));
    }
}

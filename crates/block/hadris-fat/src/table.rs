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

struct Entry<V> {
    pins: u32,
    opens: u32,
    dirty: bool,
    unlinked: bool,
    dir: bool,
    value: V,
}

/// Per-node driver state with a pin count per node, for pinned and open
/// nodes. An entry goes when its last pin does, or on `remove`.
pub(crate) struct Table<V> {
    nodes: BTreeMap<NodeId, Entry<V>>,
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
        self.nodes.get(&id).map(|entry| &entry.value)
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> Option<&mut V> {
        self.nodes.get_mut(&id).map(|entry| &mut entry.value)
    }

    /// The pin count of `id`, 0 when absent.
    pub(crate) fn pins(&self, id: NodeId) -> u32 {
        self.nodes.get(&id).map_or(0, |entry| entry.pins)
    }

    /// The first entry for which `pred` holds.
    pub(crate) fn find(&self, mut pred: impl FnMut(NodeId, &V) -> bool) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(id, entry)| pred(**id, &entry.value))
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
            .find(|(id, entry)| pred(**id, &entry.value))
            .map(|(id, _)| *id)
    }

    pub(crate) fn for_each_mut(&mut self, mut f: impl FnMut(NodeId, &mut V)) {
        for (id, entry) in &mut self.nodes {
            f(*id, &mut entry.value);
        }
    }

    /// Adds `id` with one pin, or replaces its state and keeps its pins.
    /// [`ErrorKind::LimitExceeded`] when a new entry would pass the limit.
    pub(crate) fn insert(&mut self, id: NodeId, value: V, dir: bool) -> Result<(), ErrorKind> {
        let entry = Entry {
            pins: 1,
            opens: 0,
            dirty: false,
            unlinked: false,
            dir,
            value,
        };
        if let Some(old) = self.nodes.get_mut(&id) {
            *old = Entry {
                pins: old.pins,
                ..entry
            };
            return Ok(());
        }
        if self.limit.is_some_and(|limit| self.nodes.len() >= limit) {
            return Err(ErrorKind::LimitExceeded);
        }
        self.nodes.insert(id, entry);
        Ok(())
    }

    /// Adds a pin and returns the new count, or `None` when absent.
    pub(crate) fn pin(&mut self, id: NodeId) -> Option<u32> {
        let entry = self.nodes.get_mut(&id)?;
        entry.pins = entry.pins.saturating_add(1);
        Some(entry.pins)
    }

    /// Drops a pin and returns the remaining count, or `None` when absent.
    /// The entry goes at zero.
    pub(crate) fn unpin(&mut self, id: NodeId) -> Option<u32> {
        let entry = self.nodes.get_mut(&id)?;
        entry.pins = entry.pins.saturating_sub(1);
        let pins = entry.pins;
        if pins == 0 {
            self.nodes.remove(&id);
        }
        Some(pins)
    }

    pub(crate) fn remove(&mut self, id: NodeId) -> Option<V> {
        self.nodes.remove(&id).map(|entry| entry.value)
    }

    pub(crate) fn is_dirty(&self, id: NodeId) -> bool {
        self.nodes.get(&id).is_some_and(|entry| entry.dirty)
    }

    pub(crate) fn is_unlinked(&self, id: NodeId) -> bool {
        self.nodes.get(&id).is_some_and(|entry| entry.unlinked)
    }

    pub(crate) fn first_dirty(&self) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, entry)| entry.dirty)
            .map(|(id, _)| *id)
    }

    /// Pins held by callers, not counting the driver's own pin on a dirty
    /// node.
    pub(crate) fn user_pins(&self, id: NodeId) -> u32 {
        let dirty = self.is_dirty(id);
        self.pins(id).saturating_sub(dirty as u32)
    }

    pub(crate) fn is_open(&self, id: NodeId) -> bool {
        self.nodes.get(&id).is_some_and(|entry| entry.opens > 0)
    }

    /// Marks a node dirty, taking the driver's pin on it the first time.
    pub(crate) fn mark_dirty(&mut self, id: NodeId) {
        let fresh = match self.nodes.get_mut(&id) {
            Some(node) if !node.dirty => {
                node.dirty = true;
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
        let was_dirty = match self.nodes.get_mut(&id) {
            Some(node) if node.dirty => {
                node.dirty = false;
                true
            }
            _ => false,
        };
        if was_dirty {
            self.unpin(id);
        }
    }

    /// Marks a removed node unlinked and returns its format state for cleanup
    /// while callers still pin it. Otherwise drops it.
    pub(crate) fn mark_unlinked(&mut self, id: NodeId) -> Option<&mut V> {
        self.clean(id);
        let entry = self.nodes.get_mut(&id)?;
        entry.unlinked = true;
        Some(&mut entry.value)
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
        match self.nodes.get_mut(&id) {
            Some(node) if node.unlinked => Err(ErrorKind::NotFound),
            Some(node) if node.dir => Err(ErrorKind::IsADirectory),
            Some(_) if mode == OpenMode::Write && read_only => Err(ErrorKind::ReadOnly),
            Some(node) => {
                node.opens = node.opens.saturating_add(1);
                Ok(())
            }
            None => Err(ErrorKind::InvalidHandle),
        }
    }

    /// Ends one `open`.
    pub(crate) fn close(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.opens = node.opens.saturating_sub(1);
        }
    }

    /// Drops `count` caller pins of a node; the root is ignored. A removed
    /// node leaves the table with its last pin.
    pub(crate) fn forget(&mut self, id: NodeId, count: u64) {
        if id == ROOT {
            return;
        }
        for _ in 0..count.min(u64::from(self.user_pins(id))) {
            if self.unpin(id) == Some(0) {
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
        entry: u64,
    }

    #[test]
    fn dirty_nodes_hold_one_driver_pin() {
        let mut table = Table::new(None);
        table.insert(id(2), State::default(), false).unwrap();
        table.mark_dirty(id(2));
        table.mark_dirty(id(2));
        assert_eq!(table.pins(id(2)), 2);
        assert_eq!(table.user_pins(id(2)), 1);
        table.forget(id(2), 5);
        assert_eq!(table.pins(id(2)), 1);
        assert!(table.is_dirty(id(2)));
        table.clean(id(2));
        table.clean(id(2));
        assert_eq!(table.get(id(2)), None);
    }

    #[test]
    fn changing_format_data_preserves_lifecycle_accounting() {
        let mut table = Table::new(None);
        table.insert(id(2), State { entry: 64 }, false).unwrap();
        table.open(id(2), OpenMode::Write, false).unwrap();
        table.mark_dirty(id(2));
        table.get_mut(id(2)).unwrap().entry = 96;
        table.for_each_mut(|_, node| node.entry += 32);
        assert!(table.is_dirty(id(2)));
        assert!(table.is_open(id(2)));
        assert_eq!(table.first_dirty(), Some(id(2)));
        assert_eq!(table.user_pins(id(2)), 1);
        table.forget(id(2), u64::MAX);
        assert_eq!(table.get(id(2)).map(|node| node.entry), Some(128));
        table.close(id(2));
        assert!(!table.is_open(id(2)));
        table.clean(id(2));
        assert!(table.is_empty());
    }

    #[test]
    fn opens_refuse_what_cannot_be_opened() {
        let mut table = Table::new(None);
        table.insert(id(2), State::default(), false).unwrap();
        table.insert(id(3), State::default(), true).unwrap();
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
        assert_eq!(table.nodes.get(&id(2)).map(|node| node.opens), Some(0));
    }

    #[test]
    fn unlinked_nodes_stay_until_their_last_pin() {
        let mut table = Table::new(None);
        table.insert(id(2), State { entry: 64 }, false).unwrap();
        table.insert(id(3), State::default(), false).unwrap();
        table.pin(id(2));
        table.mark_dirty(id(2));
        assert!(table.mark_unlinked(id(2)).is_some());
        let node = *table.get(id(2)).unwrap();
        assert!(table.is_unlinked(id(2)) && !table.is_dirty(id(2)));
        assert_eq!(node.entry, 64);
        assert_eq!(
            table.open(id(2), OpenMode::Read, false),
            Err(ErrorKind::NotFound)
        );
        table.forget(id(2), 1);
        assert_eq!(table.pins(id(2)), 1);
        table.forget(id(2), 1);
        assert_eq!(table.get(id(2)), None);
        table.forget(id(3), 1);
        assert!(table.mark_unlinked(id(3)).is_none());
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
        assert_eq!(table.insert(id(1), 10, false), Ok(()));
        assert_eq!(table.pins(id(1)), 1);
        assert_eq!(table.pin(id(1)), Some(2));
        assert_eq!(table.pin(id(9)), None);
        assert_eq!(table.unpin(id(1)), Some(1));
        assert_eq!(table.get(id(1)), Some(&10));
        assert_eq!(table.unpin(id(1)), Some(0));
        assert_eq!(table.get(id(1)), None);
        assert_eq!(table.unpin(id(1)), None);
        table.insert(id(2), 20, false).unwrap();
        table.insert(id(3), 30, false).unwrap();
        table.pin(id(3));
        table.insert(id(3), 31, false).unwrap();
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
        table.insert(id(1), 1, false).unwrap();
        assert_eq!(table.insert(id(2), 2, false), Err(ErrorKind::LimitExceeded));
        assert_eq!(table.insert(id(1), 3, false), Ok(()));
        table.remove(id(1));
        assert_eq!(table.insert(id(2), 2, false), Ok(()));
    }
}

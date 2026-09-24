use core::fmt;

use crate::{Error, ErrorKind, NodeId};

/// Per-node driver state keyed by [`NodeId`], with a pin count per node.
///
/// Formats without inodes (FAT, exFAT) and formats that cache node state
/// (ISO) keep one entry for every node a caller holds between `lookup` and
/// `forget`. A driver is generic over the table, `FatFs<D, T: NodeTable =
/// FixedTable<64>>`, so the user decides how nodes are stored:
/// [`FixedTable`] needs no allocator, `HeapTable` grows with `alloc`, and
/// any other type implementing this trait (an evicting cache, say) works
/// too.
///
/// The type the user names only selects the kind of table. The driver holds
/// [`With<V>`](Self::With) for its own `V`, created with
/// [`empty`](Self::empty), so its state type stays private.
///
/// # Contract
///
/// - [`insert`](Self::insert) adds a node with one pin, or fails with
///   [`TableFull`] and changes nothing.
/// - [`pin`](Self::pin) and [`unpin`](Self::unpin) move the count by one.
///   Once it reaches zero the table may drop the entry at once or later, so
///   a driver that must keep state (unwritten metadata, say) holds a pin of
///   its own until the state is safe.
/// - [`remove`](Self::remove) drops an entry whatever its count.
/// - No method panics.
pub trait NodeTable {
    /// The per-node state.
    type Value;

    /// The same kind of table holding `W`.
    type With<W>: NodeTable<Value = W>;

    /// Returns an empty table of the same kind and configuration holding `W`.
    fn empty<W>(&self) -> Self::With<W>;

    /// Returns the number of entries, pinned or not.
    fn len(&self) -> usize;

    /// Returns whether the table has no entries.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the state of `id`.
    fn get(&self, id: NodeId) -> Option<&Self::Value>;

    /// Returns the state of `id` for update.
    fn get_mut(&mut self, id: NodeId) -> Option<&mut Self::Value>;

    /// Returns the pin count of `id`, 0 when absent.
    fn pins(&self, id: NodeId) -> u32;

    /// Returns the first entry for which `pred` holds.
    fn find(&self, pred: &mut dyn FnMut(NodeId, &Self::Value) -> bool) -> Option<NodeId>;

    /// Calls `f` on every entry.
    fn for_each_mut(&mut self, f: &mut dyn FnMut(NodeId, &mut Self::Value));

    /// Adds `id` with one pin. If `id` is already present, its state is
    /// replaced and its pin count kept.
    fn insert(&mut self, id: NodeId, value: Self::Value) -> Result<(), TableFull<Self::Value>>;

    /// Adds a pin to `id` and returns the new count, or `None` when absent.
    /// The count saturates at `u32::MAX`.
    fn pin(&mut self, id: NodeId) -> Option<u32>;

    /// Drops a pin from `id` and returns the remaining count, or `None` when
    /// absent.
    fn unpin(&mut self, id: NodeId) -> Option<u32>;

    /// Removes `id` whatever its pin count and returns its state.
    fn remove(&mut self, id: NodeId) -> Option<Self::Value>;
}

/// The node table has no free slot. Holds the value that did not fit.
///
/// Converts to [`ErrorKind::LimitExceeded`].
pub struct TableFull<V>(V);

impl<V> TableFull<V> {
    /// Wraps the value that did not fit.
    pub const fn new(value: V) -> Self {
        Self(value)
    }

    /// Returns the value that did not fit.
    pub fn into_inner(self) -> V {
        self.0
    }
}

impl<V> fmt::Debug for TableFull<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TableFull")
    }
}

impl<V> fmt::Display for TableFull<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("node table is full")
    }
}

impl<V> core::error::Error for TableFull<V> {}

impl<V> From<TableFull<V>> for ErrorKind {
    fn from(_: TableFull<V>) -> Self {
        ErrorKind::LimitExceeded
    }
}

impl<V, E> From<TableFull<V>> for Error<E> {
    fn from(_: TableFull<V>) -> Self {
        Error::new(ErrorKind::LimitExceeded, "node table is full")
    }
}

struct Slot<V> {
    id: NodeId,
    pins: u32,
    value: V,
}

/// A table of at most `N` nodes in a fixed array, for drivers that must not
/// allocate. Lookups are linear, so `N` is expected to be small.
///
/// `FixedTable<N>` names the kind; a driver stores `FixedTable<N, V>`.
/// Entries are dropped when their last pin goes.
pub struct FixedTable<const N: usize, V = ()> {
    slots: [Option<Slot<V>>; N],
    len: usize,
}

impl<const N: usize, V> FixedTable<N, V> {
    /// Creates an empty table.
    pub const fn new() -> Self {
        Self {
            slots: [const { None }; N],
            len: 0,
        }
    }

    /// Returns the capacity.
    pub const fn capacity(&self) -> usize {
        N
    }

    fn slot(&self, id: NodeId) -> Option<&Slot<V>> {
        self.slots.iter().flatten().find(|slot| slot.id == id)
    }

    fn slot_mut(&mut self, id: NodeId) -> Option<&mut Slot<V>> {
        self.slots.iter_mut().flatten().find(|slot| slot.id == id)
    }
}

impl<const N: usize, V> Default for FixedTable<N, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize, V> fmt::Debug for FixedTable<N, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedTable")
            .field("len", &self.len)
            .field("capacity", &N)
            .finish_non_exhaustive()
    }
}

impl<const N: usize, V> NodeTable for FixedTable<N, V> {
    type Value = V;
    type With<W> = FixedTable<N, W>;

    fn empty<W>(&self) -> FixedTable<N, W> {
        FixedTable::new()
    }

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, id: NodeId) -> Option<&V> {
        self.slot(id).map(|slot| &slot.value)
    }

    fn get_mut(&mut self, id: NodeId) -> Option<&mut V> {
        self.slot_mut(id).map(|slot| &mut slot.value)
    }

    fn pins(&self, id: NodeId) -> u32 {
        self.slot(id).map_or(0, |slot| slot.pins)
    }

    fn find(&self, pred: &mut dyn FnMut(NodeId, &V) -> bool) -> Option<NodeId> {
        self.slots
            .iter()
            .flatten()
            .find(|slot| pred(slot.id, &slot.value))
            .map(|slot| slot.id)
    }

    fn for_each_mut(&mut self, f: &mut dyn FnMut(NodeId, &mut V)) {
        for slot in self.slots.iter_mut().flatten() {
            f(slot.id, &mut slot.value);
        }
    }

    fn insert(&mut self, id: NodeId, value: V) -> Result<(), TableFull<V>> {
        if let Some(slot) = self.slot_mut(id) {
            slot.value = value;
            return Ok(());
        }
        match self.slots.iter_mut().find(|slot| slot.is_none()) {
            Some(free) => {
                *free = Some(Slot { id, pins: 1, value });
                self.len += 1;
                Ok(())
            }
            None => Err(TableFull(value)),
        }
    }

    fn pin(&mut self, id: NodeId) -> Option<u32> {
        let slot = self.slot_mut(id)?;
        slot.pins = slot.pins.saturating_add(1);
        Some(slot.pins)
    }

    fn unpin(&mut self, id: NodeId) -> Option<u32> {
        let slot = self.slot_mut(id)?;
        slot.pins = slot.pins.saturating_sub(1);
        let pins = slot.pins;
        if pins == 0 {
            self.remove(id);
        }
        Some(pins)
    }

    fn remove(&mut self, id: NodeId) -> Option<V> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|slot| slot.id == id))?;
        self.len -= 1;
        slot.take().map(|slot| slot.value)
    }
}

/// A growable table on the heap, for mounts that hold many nodes, such as
/// a FUSE mount whose kernel keeps inodes until it forgets them.
///
/// `HeapTable` names the kind; a driver stores `HeapTable<V>`. Entries are
/// dropped when their last pin goes.
#[cfg(feature = "alloc")]
pub struct HeapTable<V = ()> {
    nodes: alloc::collections::BTreeMap<NodeId, (u32, V)>,
}

#[cfg(feature = "alloc")]
impl<V> HeapTable<V> {
    /// Creates an empty table.
    pub const fn new() -> Self {
        Self {
            nodes: alloc::collections::BTreeMap::new(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<V> Default for HeapTable<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl<V> fmt::Debug for HeapTable<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeapTable")
            .field("len", &self.nodes.len())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "alloc")]
impl<V> NodeTable for HeapTable<V> {
    type Value = V;
    type With<W> = HeapTable<W>;

    fn empty<W>(&self) -> HeapTable<W> {
        HeapTable::new()
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn get(&self, id: NodeId) -> Option<&V> {
        self.nodes.get(&id).map(|(_, value)| value)
    }

    fn get_mut(&mut self, id: NodeId) -> Option<&mut V> {
        self.nodes.get_mut(&id).map(|(_, value)| value)
    }

    fn pins(&self, id: NodeId) -> u32 {
        self.nodes.get(&id).map_or(0, |(pins, _)| *pins)
    }

    fn find(&self, pred: &mut dyn FnMut(NodeId, &V) -> bool) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(id, (_, value))| pred(**id, value))
            .map(|(id, _)| *id)
    }

    fn for_each_mut(&mut self, f: &mut dyn FnMut(NodeId, &mut V)) {
        for (id, (_, value)) in &mut self.nodes {
            f(*id, value);
        }
    }

    fn insert(&mut self, id: NodeId, value: V) -> Result<(), TableFull<V>> {
        match self.nodes.get_mut(&id) {
            Some((_, old)) => *old = value,
            None => {
                self.nodes.insert(id, (1, value));
            }
        }
        Ok(())
    }

    fn pin(&mut self, id: NodeId) -> Option<u32> {
        let (pins, _) = self.nodes.get_mut(&id)?;
        *pins = pins.saturating_add(1);
        Some(*pins)
    }

    fn unpin(&mut self, id: NodeId) -> Option<u32> {
        let (pins, _) = self.nodes.get_mut(&id)?;
        *pins = pins.saturating_sub(1);
        let pins = *pins;
        if pins == 0 {
            self.nodes.remove(&id);
        }
        Some(pins)
    }

    fn remove(&mut self, id: NodeId) -> Option<V> {
        self.nodes.remove(&id).map(|(_, value)| value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> NodeId {
        NodeId::new(raw).unwrap()
    }

    fn pins_and_removal<T: NodeTable<Value = u32>>(mut table: T) {
        assert!(table.is_empty());
        assert_eq!(
            table.insert(id(1), 10).map_err(TableFull::into_inner),
            Ok(())
        );
        assert_eq!(table.pins(id(1)), 1);
        assert_eq!(table.pin(id(1)), Some(2));
        assert_eq!(table.pin(id(9)), None);
        assert_eq!(table.unpin(id(1)), Some(1));
        assert_eq!(table.get(id(1)), Some(&10));
        assert_eq!(table.unpin(id(1)), Some(0));
        assert_eq!(table.get(id(1)), None);
        assert_eq!(table.unpin(id(1)), None);
        assert_eq!(table.pins(id(1)), 0);

        table
            .insert(id(2), 20)
            .map_err(TableFull::into_inner)
            .unwrap();
        table
            .insert(id(3), 30)
            .map_err(TableFull::into_inner)
            .unwrap();
        table.pin(id(3));
        table
            .insert(id(3), 31)
            .map_err(TableFull::into_inner)
            .unwrap();
        assert_eq!(table.pins(id(3)), 2);
        assert_eq!(table.get(id(3)), Some(&31));
        *table.get_mut(id(2)).unwrap() += 1;
        assert_eq!(table.find(&mut |_, value| *value == 21), Some(id(2)));
        let mut sum = 0;
        table.for_each_mut(&mut |_, value| {
            *value += 1;
            sum += *value;
        });
        assert_eq!(sum, 22 + 32);
        assert_eq!(table.remove(id(3)), Some(32));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn fixed_table_pins() {
        pins_and_removal(FixedTable::<4>::new().empty::<u32>());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn heap_table_pins() {
        pins_and_removal(HeapTable::<()>::new().empty::<u32>());
    }

    #[test]
    fn full_table_returns_value_and_frees_slots() {
        let mut table = FixedTable::<2, u32>::new();
        assert!(table.insert(id(1), 10).is_ok());
        assert!(table.insert(id(2), 20).is_ok());
        let full = table.insert(id(3), 30).unwrap_err();
        assert_eq!(ErrorKind::from(full), ErrorKind::LimitExceeded);
        assert_eq!(
            table.insert(id(3), 30).map_err(TableFull::into_inner),
            Err(30)
        );
        assert_eq!(table.find(&mut |_, value| *value == 20), Some(id(2)));
        assert_eq!(table.remove(id(1)), Some(10));
        assert_eq!(table.remove(id(1)), None);
        assert!(table.insert(id(3), 30).is_ok());
        assert_eq!(table.get(id(3)), Some(&30));
        assert_eq!(table.len(), 2);
        assert_eq!(table.capacity(), 2);
    }

    #[test]
    fn full_table_is_limit_exceeded() {
        let err: Error<core::convert::Infallible> = TableFull::new(()).into();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    }

    #[test]
    fn zero_capacity_table_is_always_full() {
        let mut table = FixedTable::<0, u8>::new();
        assert!(table.insert(id(1), 1).is_err());
        assert!(table.is_empty());
    }
}

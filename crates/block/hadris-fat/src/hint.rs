//! A position in a cluster chain, kept so walks resume where they left.

use hadris_fat_raw::ChainGuard;

/// A position in a chain: the cluster at index `index`, reached by a walk
/// from the first cluster that `cycle` tracks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hint {
    pub(crate) index: u32,
    /// 0 when no position is known.
    pub(crate) cluster: u32,
    pub(crate) cycle: ChainGuard,
}

impl Hint {
    pub(crate) const NONE: Self = Self {
        index: 0,
        cluster: 0,
        cycle: ChainGuard::new(0),
    };

    pub(crate) fn start(first: u32) -> Self {
        Self {
            index: 0,
            cluster: first,
            cycle: ChainGuard::new(first),
        }
    }

    /// Moves to `next`, the cluster after this one. `false` when the chain
    /// has looped.
    pub(crate) fn advance(&mut self, next: u32) -> bool {
        if !self.cycle.step(next) {
            return false;
        }
        self.cluster = next;
        self.index += 1;
        true
    }
}

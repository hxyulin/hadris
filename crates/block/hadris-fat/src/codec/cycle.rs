//! Cycle detection for cluster chains, without memory proportional to the
//! chain.

/// Brent's cycle detection over the clusters of one walk along a chain.
///
/// Feed every cluster the walk moves to, in order, to [`step`](Self::step).
/// A chain that loops is reported within about twice the length of the loop
/// after the walk enters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cycle {
    mark: u32,
    power: u32,
    steps: u32,
}

impl Cycle {
    /// A walk that starts at `first`.
    pub(crate) const fn new(first: u32) -> Self {
        Self {
            mark: first,
            power: 1,
            steps: 0,
        }
    }

    /// Records a move to `cluster`. `false` when the walk has come back to
    /// a cluster it passed, so the chain is cyclic.
    pub(crate) fn step(&mut self, cluster: u32) -> bool {
        if cluster == self.mark {
            return false;
        }
        self.steps += 1;
        if self.steps == self.power {
            self.mark = cluster;
            self.power = self.power.saturating_mul(2);
            self.steps = 0;
        }
        true
    }
}

/// A position in a chain: the cluster at index `index`, reached by a walk
/// from the first cluster that `cycle` tracks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hint {
    pub(crate) index: u32,
    /// 0 when no position is known.
    pub(crate) cluster: u32,
    pub(crate) cycle: Cycle,
}

impl Hint {
    pub(crate) const NONE: Self = Self {
        index: 0,
        cluster: 0,
        cycle: Cycle::new(0),
    };

    pub(crate) fn start(first: u32) -> Self {
        Self {
            index: 0,
            cluster: first,
            cycle: Cycle::new(first),
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

#[cfg(test)]
mod tests {
    use super::Cycle;

    fn detects(next: impl Fn(u32) -> u32, first: u32, limit: u32) -> Option<u32> {
        let mut cycle = Cycle::new(first);
        let mut cluster = first;
        for step in 1..=limit {
            cluster = next(cluster);
            if !cycle.step(cluster) {
                return Some(step);
            }
        }
        None
    }

    #[test]
    fn self_loop_is_found_at_once() {
        assert_eq!(detects(|c| c, 7, 10), Some(1));
    }

    #[test]
    fn loops_after_a_tail_are_found() {
        for tail in 0..20u32 {
            for len in 1..40u32 {
                let next = |c: u32| if c + 1 < tail + len { c + 1 } else { tail };
                let found = detects(next, 0, 4 * (tail + len) + 4).expect("loop found");
                assert!(
                    found <= 2 * (tail + 2 * len) + 2,
                    "tail {tail} len {len}: {found}"
                );
            }
        }
    }

    #[test]
    fn straight_chains_pass() {
        assert_eq!(detects(|c| c + 1, 2, 10_000), None);
    }
}

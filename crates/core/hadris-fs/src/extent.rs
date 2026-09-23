/// A byte range on a device: `len` bytes from byte `offset`.
///
/// Writers report where they stored each file with it, and a session
/// writer uses it for contents that already sit on the device it updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Extent {
    offset: u64,
    len: u64,
}

impl Extent {
    /// `len` bytes from byte `offset`.
    pub const fn new(offset: u64, len: u64) -> Self {
        Self { offset, len }
    }

    /// The first byte.
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// The length in bytes.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the range holds no bytes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The byte after the last one.
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.len)
    }
}

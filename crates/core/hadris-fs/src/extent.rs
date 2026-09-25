/// A byte range on a device: `len` bytes from byte `offset`, holding the
/// file's bytes from `file_offset`.
///
/// Writers report where they stored each file with it, a session writer
/// uses it for contents that already sit on the device it updates, and
/// drivers map files (`extents`) and locate on-disk records (`records`)
/// with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Extent {
    offset: u64,
    len: u64,
    file_offset: u64,
    unwritten: bool,
}

impl Extent {
    /// `len` bytes from byte `offset`, at file offset 0.
    pub const fn new(offset: u64, len: u64) -> Self {
        Self {
            offset,
            len,
            file_offset: 0,
            unwritten: false,
        }
    }

    /// The same range holding the file's bytes from `file_offset`.
    pub const fn with_file_offset(self, file_offset: u64) -> Self {
        Self {
            file_offset,
            ..self
        }
    }

    /// The same range, allocated but not written: it reads as zeros.
    pub const fn with_unwritten(self) -> Self {
        Self {
            unwritten: true,
            ..self
        }
    }

    /// The offset in the file of the range's first byte.
    pub const fn file_offset(&self) -> u64 {
        self.file_offset
    }

    /// Whether the range is allocated but not written.
    pub const fn is_unwritten(&self) -> bool {
        self.unwritten
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

/// A seekable, partition-relative view over a byte stream.
///
/// Implements the [`legacy`](hadris_io::legacy) traits for format crates that
/// still read streams. [`Slice`](crate::sync::Slice) replaces it as they move
/// to [`BlockDevice`](crate::sync::BlockDevice).
#[derive(Debug)]
pub struct PartitionView<'a, S> {
    pub(crate) source: &'a mut S,
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) byte_offset: u64,
    pub(crate) byte_len: u64,
    pub(crate) position: u64,
}

impl<'a, S> PartitionView<'a, S> {
    /// Creates a non-empty bounded view.
    pub fn new(
        source: &'a mut S,
        byte_offset: u64,
        byte_len: u64,
    ) -> hadris_io::legacy::Result<Self> {
        if byte_len == 0 || byte_offset.checked_add(byte_len).is_none() {
            return Err(hadris_io::legacy::Error::new(
                hadris_io::legacy::ErrorKind::InvalidInput,
                "partition view is empty or overflows",
            ));
        }
        Ok(Self {
            source,
            byte_offset,
            byte_len,
            position: 0,
        })
    }

    /// Returns the view length in bytes.
    pub const fn len(&self) -> u64 {
        self.byte_len
    }

    /// Returns whether the view is empty. Valid views are never empty.
    pub const fn is_empty(&self) -> bool {
        self.byte_len == 0
    }

    /// Returns the partition-relative cursor position.
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Releases the underlying stream borrow.
    pub fn into_inner(self) -> &'a mut S {
        self.source
    }
}

impl<S> embedded_io::ErrorType for PartitionView<'_, S> {
    type Error = hadris_io::legacy::Error;
}

#[cfg(any(feature = "sync", feature = "async"))]
impl<S> PartitionView<'_, S> {
    pub(crate) fn seek_position(
        &self,
        from: hadris_io::SeekFrom,
    ) -> hadris_io::legacy::Result<u64> {
        let position = match from {
            hadris_io::SeekFrom::Start(position) => Some(position),
            hadris_io::SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            hadris_io::SeekFrom::End(delta) => self.byte_len.checked_add_signed(delta),
        }
        .ok_or_else(|| {
            hadris_io::legacy::Error::new(
                hadris_io::legacy::ErrorKind::InvalidInput,
                "partition seek overflow",
            )
        })?;

        if position > self.byte_len {
            return Err(hadris_io::legacy::Error::new(
                hadris_io::legacy::ErrorKind::InvalidInput,
                "partition seek is out of bounds",
            ));
        }
        Ok(position)
    }

    pub(crate) fn remaining(&self) -> usize {
        usize::try_from(self.byte_len - self.position).unwrap_or(usize::MAX)
    }

    pub(crate) fn absolute_position(&self) -> hadris_io::legacy::Result<u64> {
        self.byte_offset.checked_add(self.position).ok_or_else(|| {
            hadris_io::legacy::Error::new(
                hadris_io::legacy::ErrorKind::InvalidInput,
                "partition offset overflow",
            )
        })
    }
}

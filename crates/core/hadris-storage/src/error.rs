use core::fmt;

/// Error produced by block-addressed storage operations.
#[derive(Debug)]
pub enum Error<E> {
    /// The supplied buffer is not a non-zero multiple of the logical block size.
    InvalidBufferLength {
        /// Supplied buffer length in bytes.
        length: usize,
        /// Required logical block size in bytes.
        block_size: u32,
    },
    /// The requested block range falls outside the device.
    OutOfBounds {
        /// First requested logical block.
        start: u64,
        /// Number of requested logical blocks.
        count: u64,
        /// Total logical blocks in the device.
        device_blocks: u64,
    },
    /// A block or byte-offset calculation overflowed.
    AddressOverflow,
    /// A bounded view was created with an empty or overflowing byte range.
    InvalidView {
        /// First byte in the underlying stream.
        offset: u64,
        /// Length of the view in bytes.
        length: u64,
    },
    /// The underlying byte-oriented device returned an error.
    Io(hadris_io::Error<E>),
}

/// Result returned by block-addressed storage operations.
pub type Result<T, E> = core::result::Result<T, Error<E>>;

impl<E: embedded_io::Error> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBufferLength { length, block_size } => write!(
                f,
                "buffer length {length} is not a non-zero multiple of block size {block_size}"
            ),
            Self::OutOfBounds {
                start,
                count,
                device_blocks,
            } => write!(
                f,
                "block range {start}..+{count} exceeds device size of {device_blocks} blocks"
            ),
            Self::AddressOverflow => f.write_str("block address calculation overflowed"),
            Self::InvalidView { offset, length } => {
                write!(
                    f,
                    "invalid bounded view at byte {offset} with length {length}"
                )
            }
            Self::Io(error) => write!(f, "storage I/O error: {:?}", error.kind()),
        }
    }
}

#[cfg(feature = "std")]
impl<E> std::error::Error for Error<E> where E: embedded_io::Error + fmt::Debug + 'static {}

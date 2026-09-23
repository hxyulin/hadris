use hadris_io::Result;
#[cfg(not(feature = "alloc"))]
use hadris_io::{Error, ErrorKind};

#[cfg(not(feature = "alloc"))]
const INLINE: usize = 4096;

#[derive(Debug)]
pub(crate) struct Scratch {
    #[cfg(feature = "alloc")]
    buf: alloc::vec::Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    buf: [u8; INLINE],
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
impl Scratch {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            buf: alloc::vec::Vec::new(),
            #[cfg(not(feature = "alloc"))]
            buf: [0; INLINE],
        }
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn get(&mut self, len: usize) -> Result<&mut [u8]> {
        if self.buf.len() < len {
            self.buf.resize(len, 0);
        }
        Ok(&mut self.buf[..len])
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn get(&mut self, len: usize) -> Result<&mut [u8]> {
        self.buf.get_mut(..len).ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "block size exceeds the scratch buffer",
            )
        })
    }
}

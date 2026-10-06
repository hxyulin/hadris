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

    /// A buffer of `len` bytes, or `None` if `len` exceeds the inline buffer.
    #[cfg(feature = "alloc")]
    pub(crate) fn get(&mut self, len: usize) -> Option<&mut [u8]> {
        if self.buf.len() < len {
            self.buf.resize(len, 0);
        }
        Some(&mut self.buf[..len])
    }

    /// A buffer of `len` bytes, or `None` if `len` exceeds the inline buffer.
    #[cfg(not(feature = "alloc"))]
    pub(crate) fn get(&mut self, len: usize) -> Option<&mut [u8]> {
        self.buf.get_mut(..len)
    }
}

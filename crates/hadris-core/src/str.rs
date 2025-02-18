/// A no-std compatible string type
///
/// This is a wrapper around a fixed size array of bytes
/// The string is not null terminated, and the length is stored in the struct
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FixedByteStr<const N: usize> {
    pub raw: [u8; N],
    pub len: usize,
}

impl<const N: usize> FixedByteStr<N> {
    pub fn new() -> Self {
        Self {
            raw: [0; N],
            len: 0,
        }
    }
    pub fn from_str(s: &str) -> Self {
        let mut str = Self {
            raw: [0; N],
            len: s.len(),
        };
        str.raw[..s.len()].copy_from_slice(s.as_bytes());
        str
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.raw[..self.len]).unwrap()
    }

    pub fn as_slice(&self) -> &[u8; N] {
        &self.raw
    }
}

impl<const N: usize> core::fmt::Debug for FixedByteStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("FixedByteStr")
            .field(&self.as_str())
            .finish()
    }
}

impl<const N: usize> core::fmt::Display for FixedByteStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> core::fmt::Write for FixedByteStr<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let len = s.len();
        let remaining = N.saturating_sub(self.len); 

        if len > remaining {
            return Err(core::fmt::Error); 
        }

        self.raw[self.len..self.len + len].copy_from_slice(s.as_bytes());

        self.len += len;
        Ok(())
    }
}

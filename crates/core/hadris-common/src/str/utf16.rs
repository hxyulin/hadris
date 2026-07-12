use alloc::string::String;

use crate::types::{
    endian::{Endian, LittleEndian},
    number::U16,
};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FixedUtf16Str<const N: usize> {
    data: [U16<LittleEndian>; N],
}

impl<const N: usize> FixedUtf16Str<N> {
    #[allow(clippy::result_unit_err)]
    pub fn to_string(&self) -> Result<String, ()> {
        let u16_iter = self.data.iter().map(|c| c.get());
        char::decode_utf16(u16_iter)
            .collect::<Result<String, _>>()
            .map_err(|_| ())
    }
}

#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Pod for FixedUtf16Str<N> {}
#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Zeroable for FixedUtf16Str<N> {}

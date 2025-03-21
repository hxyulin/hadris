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
    pub fn to_string(&self) -> Result<String, ()> {
        // For now we just take the lower u8 of each character
        let data = self.data.iter().map(|c| c.get() as u8).collect::<Vec<u8>>();
        String::from_utf8(data).map_err(|_| ())
    }
}

#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Pod for FixedUtf16Str<N> {}
#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Zeroable for FixedUtf16Str<N> {}

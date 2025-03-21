use crate::types::{endian::LittleEndian, number::U16};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FixedUtf16Str<const N: usize> {
    data: [U16<LittleEndian>; N],
}

#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Pod for FixedUtf16Str<N> {}
#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize> bytemuck::Zeroable for FixedUtf16Str<N> {}

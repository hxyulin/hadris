pub enum EndianType {
    NativeEndian,
    LittleEndian,
    BigEndian,
}

impl EndianType {
    pub fn read_u16(&self, bytes: [u8; 2]) -> u16 {
        match self {
            EndianType::NativeEndian => u16::from_ne_bytes(bytes),
            EndianType::LittleEndian => u16::from_le_bytes(bytes),
            EndianType::BigEndian => u16::from_be_bytes(bytes),
        }
    }

    pub fn read_u32(&self, bytes: [u8; 4]) -> u32 {
        match self {
            EndianType::NativeEndian => u32::from_ne_bytes(bytes),
            EndianType::LittleEndian => u32::from_le_bytes(bytes),
            EndianType::BigEndian => u32::from_be_bytes(bytes),
        }
    }

    pub fn write_u32(&self, value: u32, bytes: &mut [u8; 4]) {
        match self {
            EndianType::NativeEndian => bytes.copy_from_slice(&value.to_ne_bytes()),
            EndianType::LittleEndian => bytes.copy_from_slice(&value.to_le_bytes()),
            EndianType::BigEndian => bytes.copy_from_slice(&value.to_be_bytes()),
        }
    }

    pub fn u16_bytes(&self, value: u16) -> [u8; 2] {
        match self {
            EndianType::NativeEndian => value.to_ne_bytes(),
            EndianType::LittleEndian => value.to_le_bytes(),
            EndianType::BigEndian => value.to_be_bytes(),
        }
    }

    pub fn u32_bytes(&self, value: u32) -> [u8; 4] {
        match self {
            EndianType::NativeEndian => value.to_ne_bytes(),
            EndianType::LittleEndian => value.to_le_bytes(),
            EndianType::BigEndian => value.to_be_bytes(),
        }
    }
}

pub trait Endianness: Copy {
    fn get() -> EndianType;

    fn get_u16(bytes: [u8; 2]) -> u16;
    fn set_u16(value: u16, bytes: &mut [u8; 2]);
    fn get_u32(bytes: [u8; 4]) -> u32;
    fn set_u32(value: u32, bytes: &mut [u8; 4]);
    fn get_u64(bytes: [u8; 8]) -> u64;
    fn set_u64(value: u64, bytes: &mut [u8; 8]);
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, bytemuck::Zeroable, bytemuck::Pod)]
pub struct NativeEndian;
#[repr(transparent)]
#[derive(Debug, Copy, Clone, bytemuck::Zeroable, bytemuck::Pod)]
pub struct LittleEndian;
#[repr(transparent)]
#[derive(Debug, Copy, Clone, bytemuck::Zeroable, bytemuck::Pod)]
pub struct BigEndian;

impl Endianness for NativeEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::NativeEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }
}

impl Endianness for LittleEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::LittleEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }
}
impl Endianness for BigEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::BigEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }
}

pub trait Endian {
    type Output: bytemuck::Pod + bytemuck::Zeroable;
    type LsbType: bytemuck::Pod + bytemuck::Zeroable + Endian<Output = Self::Output>;
    type MsbType: bytemuck::Pod + bytemuck::Zeroable + Endian<Output = Self::Output>;

    fn new(value: Self::Output) -> Self;
    fn get(&self) -> Self::Output;
    fn set(&mut self, value: Self::Output);
}

#[cfg(all(test, feature = "std"))]
mod tests {
    #[test]
    fn test_from_le_bytes() {
        let value = u16::from_le_bytes([0x12, 0x34]);
        assert_eq!(value, 0x3412);
    }
}

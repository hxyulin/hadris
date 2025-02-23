//! Raw structures used in the FAT structures
//! The structures defined here are designed to be as compatible as possible with the FAT specification
//! wihch is why they are stored in little endian bytes instead of integer values
//! They are also repr(C, packed) to ensure correct alignment

pub mod boot_sector;
pub mod constants;
pub mod directory;
pub mod fat;
pub mod fs_info;

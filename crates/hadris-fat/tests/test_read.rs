use hadris_fat::structures::{boot_sector::BootSectorFat32, fs_info::FsInfo};

// mkfs_sectors.bin is a binary file generated using these commands:
// # Create the image (100MB)
// dd if=/dev/zero of=test.img bs=512 count=204800
// # Create the filesystem
// mkfs.fat -F 32 test.img
// # Copy first 2 sectors to the mkfs_sectors.bin file
// dd if=test.img of=mkfs_sectors.bin bs=512 count=2
const FILE_CONTENTS: &[u8] = include_bytes!("mkfs_sectors.bin");

#[test]
fn read_bs() {
    let bs_sector = &FILE_CONTENTS[0..512];
    let bs = BootSectorFat32::from_bytes(bs_sector);
    assert!(bs.jump().is_ok());
    assert_eq!(bs.oem_name().as_str(), "mkfs.fat");
    assert_eq!(bs.bytes_per_sector(), 512);
    assert_eq!(bs.sectors_per_cluster(), 1);
    assert_eq!(bs.reserved_sector_count(), 32);
    assert_eq!(bs.fat_count(), 2);

    // Sectors per fat
    // Free clusters = 204800 - 32 - sectors_per_fat * fat_count
    // sectors_per_fat = free_cluster / 128
    let sectors_per_fat = (204800 - 32) / 128;
}

#[test]
fn read_fs_info() {
    let fs_info_sector = &FILE_CONTENTS[512..1024];
    let fs_info = FsInfo::from_bytes(fs_info_sector);
    // 201615 comes from 204800 - 
    assert_eq!(fs_info.free_clusters(), 201615);
    assert_eq!(fs_info.next_free_cluster(), 2);
}

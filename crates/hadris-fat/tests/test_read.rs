// mkfs_sectors.bin is a binary file generated using these commands:
// # Create the image (100MB)
// dd if=/dev/zero of=test.img bs=512 count=204800
// # Create the filesystem
// mkfs.fat -F 32 test.img
// # Copy first 2 sectors to the mkfs_sectors.bin file
// dd if=test.img of=mkfs_sectors.bin bs=512 count=2
const FILE_CONTENTS: &[u8] = include_bytes!("mkfs_sectors.bin");

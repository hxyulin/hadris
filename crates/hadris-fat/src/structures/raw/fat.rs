//! FAT structures
//! Important notes:
//!  - The first entry (index 0) is reserved, and should contain the BPB_Media value in the lower 8
//!  bits
//!  - The second entry (index 1) should be set to the EOC value, but the higher 2 bits should be
//!  used to indicate 2 flags:
//!  - highest bit (0x8000 on FAT16, 0x08000000 on FAT32) indicates if the volume is clean, if it
//!  isn't it indicates that the volume was not properly unmounted.
//!  If this bit is not set, the driver should be scanned for any corrupted data (Bpb, FsInfo, FAT),
//!  and resort to backups if possible
//!  - the second highest bit (0x4000 on FAT16, 0x04000000 on FAT32) indicates if the volume
//!  encountered an error while reading / writing the file system the last time it was mounted.
//!  If this bit is not set , the driver should probably indicate this to the user
//!

pub mod fat16 {
    pub type ClusterEntry = [u8; 2];
}

pub mod fat32 {
    pub type ClusterEntry = [u8; 4];
}

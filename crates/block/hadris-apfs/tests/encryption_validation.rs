#![cfg(all(feature = "read", feature = "sync", feature = "alloc"))]

mod common;

use common::{BLOCK, build_image};
use hadris_apfs::sync::ApfsFs;
use hadris_apfs::types::checksum::fletcher64;
use hadris_fs::{ErrorKind, MountOptions};
use hadris_storage::{BlockSize, MemDevice};

fn set(image: &mut [u8], block: usize, offset: usize, value: u64) {
    let block = &mut image[block * BLOCK..(block + 1) * BLOCK];
    block[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    let checksum = fletcher64(block).unwrap();
    block[..8].copy_from_slice(&checksum.to_le_bytes());
}

fn device(image: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(image, BlockSize::new(512).unwrap())
}

#[test]
fn encrypted_flags_cannot_silently_mount_plaintext_looking_metadata() {
    for flags in [0, 8] {
        let mut image = build_image();
        set(&mut image, 0, 1264, 4);
        set(&mut image, 3, 264, flags);
        let error = ApfsFs::mount(device(image.clone()), MountOptions::new()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(error.into_device().into_inner(), image);
    }
}

#[cfg(feature = "encryption")]
#[test]
fn keybag_bounds_and_memory_limits_fail_before_decryption_and_retain_device() {
    for (start, blocks, expected) in [
        (u64::MAX, 1, ErrorKind::Corrupt),
        (1, u64::MAX, ErrorKind::Corrupt),
        (1, 257, ErrorKind::LimitExceeded),
    ] {
        let mut image = build_image();
        image.resize(300 * BLOCK, 0);
        set(&mut image, 0, 40, 300);
        set(&mut image, 0, 1264, 4);
        set(&mut image, 0, 1296, start);
        set(&mut image, 0, 1304, blocks);
        set(&mut image, 3, 264, 8);
        let error = ApfsFs::mount_with_password(
            device(image.clone()),
            MountOptions::new(),
            b"private-password",
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), expected);
        assert!(!format!("{error:?}").contains("private-password"));
        assert_eq!(error.into_device().into_inner(), image);
    }
}

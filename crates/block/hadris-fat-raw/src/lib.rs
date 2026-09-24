//! # hadris-fat-raw
//!
//! The on-disk layer of FAT12, FAT16, FAT32 and exFAT, for drivers, tools
//! and firmware that the `hadris-fat` driver does not fit. `hadris-fat`
//! builds on it and re-exports it as `hadris_fat::raw`.
//!
//! Everything here works on bytes and plain values, does no I/O and needs
//! no allocator:
//!
//! - the layouts of the boot sector, BPB, FSInfo sector and directory
//!   entries, with their constants;
//! - [`parse_boot`], which checks a boot sector and returns its
//!   [`Geometry`];
//! - [`FatKind`], which encodes and decodes FAT entries and classifies
//!   chain links, and [`ChainGuard`], which finds loops in a chain;
//! - [`Slot`], [`ShortEntry`] and [`LongEntry`], which decode and encode
//!   directory slots, and [`lfn`], [`short_name`] and [`name`] for long
//!   names, 8.3 names and name comparison, with [`fold_ascii`] and
//!   [`fold_unicode`] as `fn(u16) -> u16` folds;
//! - [`date`] for FAT timestamps and [`layout`] for planning a volume and
//!   encoding its boot sector;
//! - [`exfat`], the exFAT layouts and codecs.
//!
//! With the `sync`, `async` or `async-send` feature, `io` adds the FAT
//! device primitives the `hadris-fat` driver is built on, generic over a
//! `hadris-storage` block device and still allocation-free.
//!
//! The layouts mirror the specifications: they may gain items, and the
//! existing ones follow the specifications and stay exhaustive.
//!
//! ```rust
//! use hadris_fat_raw::{FatKind, Slot, lfn_checksum};
//!
//! let mut fat = [0u8; 8];
//! FatKind::Fat12.encode(3, 0xFFF, &mut fat[4..6]);
//! assert_eq!(FatKind::Fat12.decode(3, &fat[4..6]), 0xFFF);
//!
//! let mut raw = [0u8; 32];
//! raw[..11].copy_from_slice(b"README  TXT");
//! raw[11] = hadris_fat_raw::ATTR_ARCHIVE;
//! let Slot::Short(entry) = Slot::parse(&raw) else { unreachable!() };
//! assert_eq!(entry.lfn_checksum(), lfn_checksum(b"README  TXT"));
//! ```
//!
//! ## Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `sync` | No | The device primitives in `io::sync` |
//! | `async` | No | The device primitives in `io::r#async` |
//! | `async-send` | No | The device primitives with `Send` futures in `io::async_send` |
//! | `defmt` | No | `defmt::Format` for [`FatKind`] |

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![allow(clippy::duplicate_mod)]

mod boot;
mod bpb;
mod chain;
mod dirent;
mod entry;
mod slot;

pub mod date;
pub mod exfat;
#[cfg(any(feature = "sync", feature = "async"))]
pub mod io;
pub mod layout;
pub mod lfn;
pub mod name;
pub mod short_name;

pub use boot::{
    BOOT_SECTOR_LEN, BOOT_SIGNATURE, BootError, FSINFO_LEAD_SIG, FSINFO_STRUC_SIG,
    FSINFO_TRAIL_SIG, Geometry, RootLocation, check_fs_info, parse_boot,
};
pub use bpb::{BpbExt32Flags, RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};
pub use chain::ChainGuard;
pub use dirent::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_LONG_NAME, ATTR_LONG_NAME_MASK, ATTR_READ_ONLY,
    ATTR_SYSTEM, ATTR_VOLUME_ID, ENTRY_END, ENTRY_FREE, ENTRY_KANJI_E5, ENTRY_SIZE, LFN_LAST_ENTRY,
    LFN_SEQUENCE_MASK, LFN_UNITS_PER_ENTRY, NT_LOWER_BASE, NT_LOWER_EXTENSION, RawDirEntry,
    RawLfnEntry,
};
pub use entry::{
    ChainError, FAT12_MAX_CLUSTERS, FAT16_MAX_CLUSTERS, FAT32_MAX_CLUSTERS, FIRST_DATA_CLUSTER,
    FatKind,
};
pub use lfn::checksum as lfn_checksum;
pub use name::{fold_ascii, fold_unicode};
pub use slot::{LongEntry, MAX_DIR_ENTRIES, ShortEntry, Slot};

//! exFAT on-disk layouts and codecs.
//!
//! The layouts and constants mirror the exFAT 1.00 specification: they may
//! gain items, and the existing ones follow the specification and stay
//! exhaustive. The codecs check a boot sector into a [`Geometry`], compute
//! the boot, set and up-case table checksums and name hashes, encode names
//! and times, and decode compressed up-case tables. With a mode feature,
//! `io` adds the device primitives.

mod codec;
mod detail;
#[cfg(any(feature = "sync", feature = "async"))]
pub mod io;
mod layout;

pub use codec::{
    Geometry, MAX_SET, NameUnits, PageStart, RawEntry, UpcaseDecoder, boot_checksum, decode_time,
    encode_time, hash_unit, mandatory_upcase, name_hash, parse_boot, seal, set_checksum,
    table_checksum, valid_unit,
};
pub use detail::Detail;
pub use layout::{
    ALLOCATION_POSSIBLE, ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_READ_ONLY, ATTR_SYSTEM,
    BOOT_REGION_SECTORS, BOOT_SIGNATURE, BitmapEntry, BootSector, CATEGORY_SECONDARY,
    CHECKSUM_SKIPPED, ENTRY_BITMAP, ENTRY_END, ENTRY_FILE, ENTRY_GUID, ENTRY_LABEL, ENTRY_NAME,
    ENTRY_PADDING, ENTRY_SIZE, ENTRY_STREAM, ENTRY_UPCASE, ENTRY_VENDOR_ALLOCATION,
    ENTRY_VENDOR_EXTENSION, EXTENDED_BOOT_SIGNATURE, FAT_BAD, FAT_END, FAT_MEDIA, FILE_SYSTEM_NAME,
    FIRST_CLUSTER, FileEntry, IMPORTANCE_BENIGN, IN_USE, JUMP_BOOT, LabelEntry, MAX_CLUSTER_COUNT,
    MAX_DIRECTORY_SIZE, MAX_LABEL_UNITS, MAX_NAME_UNITS, NAME_UNITS_PER_ENTRY, NO_FAT_CHAIN,
    NameEntry, RECOMMENDED_UPCASE_CHECKSUM, RECOMMENDED_UPCASE_TABLE, StreamEntry,
    UTC_OFFSET_VALID, UpcaseEntry, VOLUME_ACTIVE_FAT, VOLUME_CLEAR_TO_ZERO, VOLUME_DIRTY,
    VOLUME_MEDIA_FAILURE,
};

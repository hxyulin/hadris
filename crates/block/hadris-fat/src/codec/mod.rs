//! Mode-independent encoding and decoding of FAT on-disk values.
//!
//! Everything here works on bytes and plain values and performs no I/O, so
//! the legacy `FatVolume` and the V3 driver share it.

pub(crate) mod boot;
pub(crate) mod date;
pub(crate) mod entry;
pub(crate) mod lfn;
pub(crate) mod short_name;

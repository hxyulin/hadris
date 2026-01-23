//! # Hadris ISO
//!
//! A comprehensive Rust implementation of the ISO 9660 filesystem with support for
//! Joliet, Rock Ridge (RRIP), El-Torito booting, and no-std environments.
//!
//! This crate provides both reading and writing capabilities for ISO 9660 images,
//! making it suitable for:
//! - **Bootloaders**: Minimal no-std + no-alloc read support
//! - **OS Kernels**: Read ISO filesystems with only a heap allocator
//! - **Desktop Applications**: Full-featured ISO creation and extraction
//! - **Build Systems**: Automated bootable ISO generation
//!
//! ## Quick Start
//!
//! ### Reading an ISO Image
//!
//! ```rust,no_run
//! use std::fs::File;
//! use std::io::BufReader;
//! use hadris_iso::read::IsoImage;
//!
//! let file = File::open("image.iso").unwrap();
//! let reader = BufReader::new(file);
//! let image = IsoImage::open(reader).unwrap();
//!
//! // Get the root directory
//! let root = image.root_dir();
//!
//! // Iterate through files
//! for entry in root.iter(&image).entries() {
//!     let entry = entry.unwrap();
//!     println!("File: {:?}", String::from_utf8_lossy(entry.name()));
//! }
//! ```
//!
//! ### Creating a Bootable ISO
//!
//! ```rust,no_run
//! use std::io::Cursor;
//! use std::sync::Arc;
//! use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
//! use hadris_iso::boot::EmulationType;
//! use hadris_iso::read::PathSeparator;
//! use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
//! use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
//!
//! // Prepare files to include
//! let boot_image = std::fs::read("boot.bin").unwrap();
//! let files = InputFiles {
//!     path_separator: PathSeparator::ForwardSlash,
//!     files: vec![
//!         IsoFile::File {
//!             name: Arc::new("boot.bin".to_string()),
//!             contents: boot_image,
//!         },
//!     ],
//! };
//!
//! // Configure boot options
//! let boot_options = BootOptions {
//!     write_boot_catalog: true,
//!     default: BootEntryOptions {
//!         boot_image_path: "boot.bin".to_string(),
//!         load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
//!         boot_info_table: false,
//!         grub2_boot_info: false,
//!         emulation: EmulationType::NoEmulation,
//!     },
//!     entries: vec![],
//! };
//!
//! // Create the ISO
//! let format_options = FormatOptions {
//!     volume_name: "MY_BOOTABLE_ISO".to_string(),
//!     sector_size: 2048,
//!     path_seperator: PathSeparator::ForwardSlash,
//!     features: CreationFeatures {
//!         filenames: BaseIsoLevel::Level1 {
//!             supports_lowercase: false,
//!             supports_rrip: false,
//!         },
//!         long_filenames: false,
//!         joliet: None,
//!         rock_ridge: None,
//!         el_torito: Some(boot_options),
//!         hybrid_boot: None,
//!     },
//! };
//!
//! let mut buffer = Cursor::new(vec![0u8; 1024 * 1024]); // 1MB buffer
//! IsoImageWriter::format_new(&mut buffer, files, format_options).unwrap();
//! std::fs::write("bootable.iso", buffer.into_inner()).unwrap();
//! ```
//!
//! ## Feature Flags
//!
//! This crate uses feature flags to control functionality and dependencies:
//!
//! | Feature | Description | Dependencies |
//! |---------|-------------|--------------|
//! | `read` | Minimal read support (no-std, no-alloc) | None |
//! | `alloc` | Heap allocation without full std | `alloc` crate |
//! | `std` | Full standard library support | `std`, `alloc`, `thiserror`, `tracing`, `chrono` |
//! | `write` | ISO creation/formatting | `std`, `alloc` |
//! | `joliet` | UTF-16 Unicode filename support | `alloc` |
//!
//! ### Feature Combinations
//!
//! **For Bootloaders (minimal footprint):**
//! ```toml
//! [dependencies]
//! hadris-iso = { version = "0.2", default-features = false, features = ["read"] }
//! ```
//!
//! **For Kernels with Heap (no-std + alloc):**
//! ```toml
//! [dependencies]
//! hadris-iso = { version = "0.2", default-features = false, features = ["read", "alloc"] }
//! ```
//!
//! **For Desktop Applications (full features):**
//! ```toml
//! [dependencies]
//! hadris-iso = { version = "0.2" }  # Uses default features: std, write
//! ```
//!
//! ## ISO 9660 Extensions
//!
//! ### Joliet Extension
//!
//! Joliet provides Unicode filename support using UTF-16 encoding. It allows
//! filenames up to 64 characters and preserves case. Enable with the `joliet` feature.
//!
//! ```rust,ignore
//! use hadris_iso::joliet::JolietLevel;
//! use hadris_iso::write::options::CreationFeatures;
//!
//! let features = CreationFeatures {
//!     joliet: Some(JolietLevel::Level3), // Full Unicode support
//!     ..Default::default()
//! };
//! ```
//!
//! ### Rock Ridge (RRIP) Extension
//!
//! Rock Ridge provides POSIX filesystem semantics including:
//! - Long filenames (up to 255 characters)
//! - Unix permissions and ownership
//! - Symbolic links
//! - Device files
//!
//! ### El-Torito Boot Extension
//!
//! El-Torito enables bootable CD/DVD images. This crate supports:
//! - BIOS boot (x86/x86_64)
//! - UEFI boot
//! - No-emulation boot mode
//! - Boot information table injection
//!
//! ### Hybrid Boot (USB Boot)
//!
//! Hybrid boot enables ISOs to be bootable when written directly to USB drives:
//! - **MBR mode** - For BIOS systems (isohybrid-compatible)
//! - **GPT mode** - For UEFI systems
//! - **Hybrid MBR+GPT** - For dual BIOS/UEFI compatibility
//!
//! ```rust,ignore
//! use hadris_iso::write::options::{HybridBootOptions, PartitionScheme};
//!
//! // Enable MBR-based hybrid boot for USB
//! let features = CreationFeatures {
//!     hybrid_boot: Some(HybridBootOptions::mbr()),
//!     ..Default::default()
//! };
//!
//! // Enable dual BIOS/UEFI boot
//! let features = CreationFeatures {
//!     hybrid_boot: Some(HybridBootOptions::hybrid()),
//!     ..Default::default()
//! };
//! ```
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`boot`] - El-Torito boot catalog structures and options
//! - [`directory`] - Directory record parsing and creation
//! - [`file`] - File entry types and filename handling
//! - [`io`] - Sector-based I/O abstractions
//! - [`joliet`] - Joliet UTF-16 extension support
//! - [`path`] - Path table structures
//! - [`read`] - ISO image reading and navigation
//! - [`rrip`] - Rock Ridge extension support
//! - [`susp`] - System Use Sharing Protocol (base for Rock Ridge)
//! - [`types`] - Common types (endian values, strings, dates)
//! - [`volume`] - Volume descriptor structures
//! - [`write`] - ISO image creation
//!
//! ## Compatibility
//!
//! ISOs created with this crate are compatible with:
//! - Linux (mount, isoinfo)
//! - Windows (built-in ISO support)
//! - macOS (built-in ISO support)
//! - QEMU/VirtualBox (bootable ISOs)
//! - xorriso (can read/verify)
//!
//! ## Specification References
//!
//! This implementation follows these specifications:
//! - ECMA-119 (ISO 9660)
//! - Joliet Specification (Microsoft)
//! - IEEE P1282 (Rock Ridge / RRIP)
//! - El-Torito Bootable CD-ROM Format Specification
//!
//! For detailed specification documentation, see the
//! [spec directory](https://github.com/hxyulin/hadris/tree/main/crates/hadris-iso/spec).

// Known Bugs:
//  - Zero size files causes a lot of issues
//
//  TODO: There is a lot of bugs with mixing file interchanges!!!

#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

/// Directory record structures for parsing and creating directory entries.
///
/// This module provides the [`DirectoryRecord`] type which represents a single
/// entry in an ISO 9660 directory. Each record contains metadata about a file
/// or subdirectory including its location, size, timestamps, and flags.
///
/// # Example
///
/// ```rust,ignore
/// use hadris_iso::directory::{DirectoryRecord, FileFlags};
///
/// // Check if a record is a directory
/// let flags = FileFlags::from_bits_truncate(record.header().flags);
/// if flags.contains(FileFlags::DIRECTORY) {
///     println!("This is a directory");
/// }
/// ```
pub mod directory;

/// File entry types and interchange levels.
///
/// ISO 9660 defines three interchange levels with different filename restrictions:
/// - **Level 1**: 8.3 format (8 chars + 3 extension), uppercase only
/// - **Level 2**: Up to 31 characters
/// - **Level 3**: Up to 207 characters
///
/// This module also handles the [`EntryType`] enum which tracks which
/// extensions (Joliet, Rock Ridge) are available for a given entry.
pub mod file;

/// Sector-based I/O abstractions for reading and writing ISOs.
///
/// ISO 9660 uses 2048-byte sectors as its fundamental unit. This module
/// provides [`IsoCursor`] which wraps any `Read + Seek` type and provides
/// sector-aligned operations.
pub mod io;

/// Path table structures for fast directory lookup.
///
/// The path table provides an alternative to traversing the directory
/// hierarchy for locating directories. It's a flat list of all directories
/// in the image, useful for quick lookups.
pub mod path;

/// Common types used throughout the crate.
///
/// This includes:
/// - Endian-aware integer types ([`U16`], [`U32`], [`BothEndian`])
/// - ISO 9660 string types with character set restrictions
/// - Date/time structures ([`DecDateTime`], [`BinDateTime`])
pub mod types;

/// Volume descriptor structures.
///
/// Volume descriptors are located starting at sector 16 and describe the
/// overall structure of the ISO image. Types include:
/// - [`PrimaryVolumeDescriptor`] - Required, contains basic image info
/// - [`SupplementaryVolumeDescriptor`] - For Joliet and other extensions
/// - [`BootRecordVolumeDescriptor`] - For El-Torito boot support
pub mod volume;

/// El-Torito boot support.
///
/// This module provides structures and utilities for creating and parsing
/// bootable ISO images according to the El-Torito specification.
///
/// # Boot Catalog Structure
///
/// The boot catalog consists of:
/// 1. **Validation Entry** - Identifies the catalog as valid
/// 2. **Default/Initial Entry** - The primary boot image
/// 3. **Section Headers** - For additional boot images (optional)
/// 4. **Section Entries** - Additional boot images (UEFI, etc.)
///
/// # Example
///
/// ```rust,ignore
/// use hadris_iso::boot::{BootCatalog, EmulationType, PlatformId};
///
/// let catalog = BootCatalog::new(
///     EmulationType::NoEmulation,
///     0,      // load_segment (0 = default 0x07C0)
///     4,      // sector_count (512-byte sectors to load)
///     20,     // load_rba (LBA of boot image)
/// );
/// ```
pub mod boot;

/// Joliet extension for Unicode filenames.
///
/// Joliet uses UTF-16 Big Endian encoding and supports filenames up to
/// 64 characters (128 bytes). It's widely supported on Windows and Linux.
///
/// Three levels are defined:
/// - **Level 1**: Escape sequence `%/@`
/// - **Level 2**: Escape sequence `%/C`
/// - **Level 3**: Escape sequence `%/E` (recommended)
#[cfg(feature = "alloc")]
pub mod joliet;

/// Rock Ridge Interchange Protocol (RRIP) extension.
///
/// Rock Ridge provides POSIX filesystem semantics on top of ISO 9660,
/// including long filenames, Unix permissions, symbolic links, and more.
///
/// # Supported Extensions
///
/// - `PX` - POSIX file attributes (mode, nlink, uid, gid)
/// - `NM` - Alternate (long) filename
/// - `SL` - Symbolic link
/// - `TF` - Timestamps (creation, modification, access)
/// - `CL`/`PL`/`RE` - Directory relocation
#[cfg(feature = "std")]
pub mod rrip;

/// System Use Sharing Protocol (SUSP).
///
/// SUSP provides a framework for extending ISO 9660 directory records
/// with additional system-specific information. Rock Ridge is built on SUSP.
#[cfg(feature = "std")]
pub mod susp;

/// ISO image reading and navigation.
///
/// This module provides the main [`IsoImage`] type for opening and
/// navigating ISO 9660 images.
///
/// # Example
///
/// ```rust,ignore
/// use std::fs::File;
/// use hadris_iso::read::IsoImage;
///
/// let file = File::open("image.iso")?;
/// let image = IsoImage::open(file)?;
///
/// // Read the primary volume descriptor
/// let pvd = image.read_pvd();
/// println!("Volume: {}", pvd.volume_identifier.to_str().trim());
///
/// // Navigate directories
/// let root = image.root_dir();
/// for entry in root.iter(&image).entries() {
///     // Process each entry
/// }
/// ```
#[cfg(feature = "alloc")]
pub mod read;

/// ISO image creation and formatting.
///
/// This module provides [`IsoImageWriter`] for creating new ISO images
/// with full support for El-Torito boot, Joliet, and Rock Ridge extensions.
///
/// # Example
///
/// ```rust,ignore
/// use hadris_iso::write::{IsoImageWriter, InputFiles, FormatOptions};
///
/// let files = InputFiles::from_fs(&path, PathSeparator::ForwardSlash)?;
/// let options = FormatOptions::default();
///
/// let mut output = File::create("output.iso")?;
/// IsoImageWriter::format_new(&mut output, files, options)?;
/// ```
#[cfg(feature = "write")]
pub mod write;

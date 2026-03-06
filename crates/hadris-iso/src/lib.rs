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
//! ```rust
//! # use std::io::Cursor;
//! # use std::sync::Arc;
//! # use hadris_iso::read::PathSeparator;
//! # use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
//! # use hadris_iso::write::options::{FormatOptions, CreationFeatures};
//! use hadris_iso::read::IsoImage;
//!
//! # // Create a minimal ISO image for the example
//! # let files = InputFiles {
//! #     path_separator: PathSeparator::ForwardSlash,
//! #     files: vec![
//! #         IsoFile::File {
//! #             name: Arc::new("readme.txt".to_string()),
//! #             contents: b"Hello, World!".to_vec(),
//! #         },
//! #     ],
//! # };
//! # let options = FormatOptions {
//! #     volume_name: "TEST".to_string(),
//! #     system_id: None, volume_set_id: None, publisher_id: None,
//! #     preparer_id: None, application_id: None,
//! #     sector_size: 2048,
//! #     path_separator: PathSeparator::ForwardSlash,
//! #     features: CreationFeatures::default(),
//! # };
//! # let mut buffer = Cursor::new(vec![0u8; 1024 * 1024]);
//! # IsoImageWriter::format_new(&mut buffer, files, options).unwrap();
//! # let reader = Cursor::new(buffer.into_inner());
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
//! ```rust
//! use std::io::Cursor;
//! use std::sync::Arc;
//! use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
//! use hadris_iso::boot::EmulationType;
//! use hadris_iso::read::PathSeparator;
//! use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
//! use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
//!
//! // Prepare files to include (use dummy boot image for example)
//! # let boot_image = vec![0u8; 2048]; // Minimal boot image
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
//!     system_id: None, volume_set_id: None, publisher_id: None,
//!     preparer_id: None, application_id: None,
//!     sector_size: 2048,
//!     path_separator: PathSeparator::ForwardSlash,
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
//! let mut buffer = Cursor::new(vec![0u8; 2 * 1024 * 1024]); // 2MB buffer
//! IsoImageWriter::format_new(&mut buffer, files, format_options).unwrap();
//! # // In real code you would write to a file:
//! # // std::fs::write("bootable.iso", buffer.into_inner()).unwrap();
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
//! ```rust
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
//! ```rust
//! use hadris_iso::write::options::{CreationFeatures, HybridBootOptions, PartitionScheme};
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
//! - [`mod@file`] - File entry types and filename handling
//! - [`io`] - Sector-based I/O abstractions
//! - [`joliet`] - Joliet UTF-16 extension support
//! - [`path`] - Path table structures
//! - [`read`] - ISO image reading and navigation
//! - [`rrip`] - Rock Ridge extension support
//! - [`susp`] - System Use Sharing Protocol (base for Rock Ridge)
//! - [`types`] - Common types (endian values, strings, dates)
//! - [`volume`] - Volume descriptor structures
//! - [`mod@write`] - ISO image creation
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

// Known Limitations:
//  - Rock Ridge write support is not yet implemented (read support works)
//  - When reading ISOs with both Joliet and Rock Ridge, only one is used

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

/// File entry types and interchange levels.
///
/// ISO 9660 defines three interchange levels with different filename restrictions:
/// - **Level 1**: 8.3 format (8 chars + 3 extension), uppercase only
/// - **Level 2**: Up to 31 characters
/// - **Level 3**: Up to 207 characters
///
/// This module also handles the `EntryType` enum which tracks which
/// extensions (Joliet, Rock Ridge) are available for a given entry.
pub mod file;

/// Common types used throughout the crate.
///
/// This includes:
/// - Endian-aware integer types (`U16`, `U32`, `BothEndian`)
/// - ISO 9660 string types with character set restrictions
/// - Date/time structures (`DecDateTime`, `BinDateTime`)
pub mod types;

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

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous ISO 9660 API.
    //!
    //! All I/O operations use synchronous `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::sync::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    macro_rules! sync_only {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! async_only {
        ($($item:tt)*) => {};
    }

    #[path = "."]
    mod __inner {
        /// Sector-based I/O abstractions for reading and writing ISOs.
        ///
        /// ISO 9660 uses 2048-byte sectors as its fundamental unit. This module
        /// provides `IsoCursor` which wraps any `Read + Seek` type and provides
        /// sector-aligned operations.
        pub mod io;

        /// Directory record structures for parsing and creating directory entries.
        ///
        /// This module provides the `DirectoryRecord` type which represents a single
        /// entry in an ISO 9660 directory. Each record contains metadata about a file
        /// or subdirectory including its location, size, timestamps, and flags.
        ///
        /// # Example
        ///
        /// ```rust
        /// use hadris_iso::directory::FileFlags;
        ///
        /// // Check if flags indicate a directory
        /// let flags = FileFlags::from_bits_truncate(0x02); // DIRECTORY flag
        /// if flags.contains(FileFlags::DIRECTORY) {
        ///     println!("This is a directory");
        /// }
        /// ```
        pub mod directory;

        /// Volume descriptor structures.
        ///
        /// Volume descriptors are located starting at sector 16 and describe the
        /// overall structure of the ISO image. Types include:
        /// - `PrimaryVolumeDescriptor` - Required, contains basic image info
        /// - `SupplementaryVolumeDescriptor` - For Joliet and other extensions
        /// - `BootRecordVolumeDescriptor` - For El-Torito boot support
        pub mod volume;

        /// Path table structures for fast directory lookup.
        ///
        /// The path table provides an alternative to traversing the directory
        /// hierarchy for locating directories. It's a flat list of all directories
        /// in the image, useful for quick lookups.
        pub mod path;

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
        /// ```rust
        /// use hadris_iso::boot::{BootCatalog, EmulationType};
        ///
        /// let catalog = BootCatalog::new(
        ///     EmulationType::NoEmulation,
        ///     0,      // load_segment (0 = default 0x07C0)
        ///     4,      // sector_count (512-byte sectors to load)
        ///     20,     // load_rba (LBA of boot image)
        /// );
        /// ```
        pub mod boot;

        /// System Use Sharing Protocol (SUSP).
        ///
        /// SUSP provides a framework for extending ISO 9660 directory records
        /// with additional system-specific information. Rock Ridge is built on SUSP.
        #[cfg(feature = "alloc")]
        pub mod susp;

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
        #[cfg(feature = "alloc")]
        pub mod rrip;

        /// ISO image reading and navigation.
        ///
        /// This module provides the main `IsoImage` type for opening and
        /// navigating ISO 9660 images.
        ///
        /// # Example
        ///
        /// ```rust
        /// # use std::io::Cursor;
        /// # use std::sync::Arc;
        /// # use hadris_iso::read::PathSeparator;
        /// # use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
        /// # use hadris_iso::write::options::{FormatOptions, CreationFeatures};
        /// use hadris_iso::read::IsoImage;
        ///
        /// # // Create a minimal ISO for the example
        /// # let files = InputFiles {
        /// #     path_separator: PathSeparator::ForwardSlash,
        /// #     files: vec![IsoFile::File {
        /// #         name: Arc::new("test.txt".to_string()),
        /// #         contents: b"test".to_vec(),
        /// #     }],
        /// # };
        /// # let options = FormatOptions {
        /// #     volume_name: "TEST".to_string(),
        /// #     system_id: None, volume_set_id: None, publisher_id: None,
        /// #     preparer_id: None, application_id: None,
        /// #     sector_size: 2048,
        /// #     path_separator: PathSeparator::ForwardSlash,
        /// #     features: CreationFeatures::default(),
        /// # };
        /// # let mut buffer = Cursor::new(vec![0u8; 1024 * 1024]);
        /// # IsoImageWriter::format_new(&mut buffer, files, options).unwrap();
        /// # let file = Cursor::new(buffer.into_inner());
        /// let image = IsoImage::open(file).unwrap();
        ///
        /// // Read the primary volume descriptor
        /// let pvd = image.read_pvd();
        /// println!("Volume: {}", pvd.volume_identifier.to_str().trim());
        ///
        /// // Navigate directories
        /// let root = image.root_dir();
        /// for entry in root.iter(&image).entries() {
        ///     // Process each entry
        /// #   let _ = entry;
        /// }
        /// ```
        #[cfg(feature = "alloc")]
        pub mod read;

        /// ISO image creation and formatting.
        ///
        /// This module provides `IsoImageWriter` for creating new ISO images
        /// with full support for El-Torito boot, Joliet, and Rock Ridge extensions.
        ///
        /// # Example
        ///
        /// ```rust
        /// use std::io::Cursor;
        /// use std::sync::Arc;
        /// use hadris_iso::read::PathSeparator;
        /// use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
        /// use hadris_iso::write::options::{FormatOptions, CreationFeatures};
        ///
        /// // Create files to include in the ISO
        /// let files = InputFiles {
        ///     path_separator: PathSeparator::ForwardSlash,
        ///     files: vec![
        ///         IsoFile::File {
        ///             name: Arc::new("hello.txt".to_string()),
        ///             contents: b"Hello, World!".to_vec(),
        ///         },
        ///     ],
        /// };
        /// let options = FormatOptions {
        ///     volume_name: "MY_ISO".to_string(),
        ///     system_id: None, volume_set_id: None, publisher_id: None,
        ///     preparer_id: None, application_id: None,
        ///     sector_size: 2048,
        ///     path_separator: PathSeparator::ForwardSlash,
        ///     features: CreationFeatures::default(),
        /// };
        ///
        /// let mut output = Cursor::new(vec![0u8; 1024 * 1024]);
        /// IsoImageWriter::format_new(&mut output, files, options).unwrap();
        /// # // In real code, write to a file instead of a Cursor
        /// ```
        #[cfg(feature = "write")]
        pub mod write;

        /// ISO image modification and append support.
        ///
        /// This module provides `IsoModifier` for appending files to existing ISO images
        /// and marking files for deletion. Changes are committed as a new session.
        ///
        /// # Example
        ///
        /// ```rust,ignore
        /// use hadris_iso::modify::IsoModifier;
        ///
        /// let file = std::fs::OpenOptions::new()
        ///     .read(true).write(true)
        ///     .open("image.iso")?;
        ///
        /// let mut modifier = IsoModifier::open(file)?;
        /// modifier.append_file("new_file.txt", b"Hello, world!".to_vec());
        /// modifier.commit()?;
        /// ```
        #[cfg(feature = "write")]
        pub mod modify;
    }
    pub use __inner::*;

    // Convenience re-exports for backwards compatibility
    pub use __inner::io::IsoCursor;
    #[cfg(feature = "alloc")]
    pub use __inner::read::IsoImage;
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous ISO 9660 API.
    //!
    //! All I/O operations use async `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::r#async::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! sync_only {
        ($($item:tt)*) => {};
    }

    macro_rules! async_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[path = "."]
    mod __inner {
        pub mod boot;
        pub mod directory;
        pub mod io;
        #[cfg(feature = "write")]
        pub mod modify;
        pub mod path;
        #[cfg(feature = "alloc")]
        pub mod read;
        #[cfg(feature = "alloc")]
        pub mod rrip;
        #[cfg(feature = "alloc")]
        pub mod susp;
        pub mod volume;
        #[cfg(feature = "write")]
        pub mod write;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

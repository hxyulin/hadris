# Hadris ISO

A pure Rust ISO 9660 filesystem and ISO image library with allocation-free
reading plus full-featured read/write support for Joliet, Rock Ridge (RRIP),
SUSP, El Torito, and ISO 9660:1999. Hadris ISO is designed for desktop
applications and `no_std` bootloaders, operating-system kernels, firmware, and
embedded systems working with CD-ROM, DVD, and bootable optical-disc images.

## Features

- **Read & Write Support** - Full-featured ISO creation and extraction
- **Zero-allocation Reader** - Navigate ISO 9660 and Joliet trees and stream
  multi-extent files entirely through caller-owned buffers
- **No-std Compatible** - Use the sync or async reader in bootloaders, firmware,
  and custom kernels without a global allocator
- **El-Torito Boot** - Create bootable CD/DVD images for BIOS systems
- **Joliet Extension** - UTF-16 Unicode filenames (up to 64 characters)
- **Rock Ridge (RRIP) Extension** - POSIX filesystem semantics (long names, permissions, symlinks)
- **SUSP (System Use Sharing Protocol)** - Standardized extension framework
- **ISO 9660:1999** - Long filenames up to 207 characters with proper Level 2/3 compliance

## Quick Start

### Reading an ISO

```rust
use std::fs::File;
use std::io::BufReader;
use hadris_iso::read::IsoImage;

let file = File::open("image.iso")?;
let reader = BufReader::new(file);
let image = IsoImage::open(reader)?;

// Iterate through root directory
let root = image.root_dir();
for entry in root.iter(&image).entries() {
    let entry = entry?;
    println!("File: {:?}", String::from_utf8_lossy(entry.name()));
}
```

### Creating a Bootable ISO

```rust
use std::io::Cursor;
use std::sync::Arc;
use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
use hadris_iso::boot::EmulationType;
use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};

// Prepare files
let files = InputFiles {
    path_separator: PathSeparator::ForwardSlash,
    files: vec![
        IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_image_bytes,
        },
    ],
};

// Configure boot options
let boot_options = BootOptions {
    write_boot_catalog: true,
    default: BootEntryOptions {
        boot_image_path: "boot.bin".to_string(),
        load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
        boot_info_table: false,
        grub2_boot_info: false,
        emulation: EmulationType::NoEmulation,
    },
    entries: vec![],
};

// Create ISO
let format_options = IsoFormatOptions {
    volume_name: "BOOTABLE".to_string(),
    sector_size: 2048,
    path_separator: PathSeparator::ForwardSlash,
    features: CreationFeatures {
        filenames: BaseIsoLevel::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        },
        long_filenames: false,
        joliet: None,
        rock_ridge: None,
        el_torito: Some(boot_options),
        ..CreationFeatures::default()
    },
    system_id: None,
    volume_set_id: None,
    publisher_id: None,
    preparer_id: None,
    application_id: None,
    strict_charset: false,
};

let mut buffer = Cursor::new(vec![0u8; 1024 * 1024]);
IsoImageWriter::create(&mut buffer, files, format_options)?;
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Allocation-free ISO 9660/Joliet navigation and streamed file reads | No heap allocator |
| `alloc` | Owned collections, names, RRIP enrichment, and convenience reads | `read`, `alloc` crate |
| `std` | Full standard library support | `std`, `alloc` |
| `sync` | Synchronous API under `hadris_iso::sync` | — |
| `async` | Asynchronous read API under `hadris_iso::r#async` | — |
| `write` | Synchronous ISO creation/formatting | `std`, `alloc` |
| `joliet` | Allocating Joliet encode/write helpers; allocation-free Joliet reading is part of `read` | `alloc` |

`std` selects platform integration but does not select an I/O mode. The default
configuration enables `sync`; custom configurations should select `sync`,
`async`, or both explicitly. Write and modification APIs are currently available
only under `sync`.

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-iso = { version = "2.0.0-rc.4", default-features = false, features = ["read", "sync"] }
```

The `read` feature exposes `IsoReader`, which opens and navigates ISO 9660 and
Joliet trees without heap allocation. It performs nested path lookup, groups
multi-extent files, and streams file contents into caller-owned buffers in
both synchronous and asynchronous configurations:

```rust
use hadris_iso::read::IsoReader;

let mut image = IsoReader::open(device)?;
if let Some(entry) = image.find_path("BOOT/KERNEL.BIN")? {
    let mut file = image.open_file(&entry)?;
    while file.read_chunk(&mut scratch)? != 0 {
        // Consume the initialized part of `scratch`.
    }
}
# Ok::<(), hadris_iso::Error>(())
```

The reader prefers the highest recognized Joliet namespace. Use
`primary_root()` with `find_path_in()` when raw ISO 9660 naming is required.
Rock Ridge enrichment remains part of the allocation-backed `IsoImage` API;
the allocation-free reader exposes raw system-use bytes for custom handling.

### For Kernels with Heap (no-std + alloc)

```toml
[dependencies]
hadris-iso = { version = "2.0.0-rc.4", default-features = false, features = ["read", "alloc", "sync"] }
```

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-iso = "2.0.0-rc.4"  # Uses default features
```

## Extension Support

| Extension | Read | Write | Notes |
|-----------|------|-------|-------|
| ISO 9660 Level 1-3 | Yes | Yes | Allocation-free navigation and multi-extent streaming available |
| ISO 9660:1999 | Yes | Yes | Long filenames up to 207 chars (Level 2/3 compliance) |
| SUSP | Yes | Yes | System Use Sharing Protocol for extension framework |
| Joliet | Yes | Yes | Allocation-free UTF-16BE lookup/decoding; owned helpers with `alloc` |
| Rock Ridge (RRIP) | Yes | Yes | Allocation-backed metadata enrichment; raw system-use bytes remain available without `alloc` |
| El-Torito | Yes | Yes | BIOS bootable images |
| Hybrid Boot (MBR/GPT) | - | Yes | USB bootable images (MBR, GPT, or dual) |

## Comparison with Other Tools

| Feature | hadris-iso | cdfs | iso9660-rs | xorriso |
|---------|------------|------|------------|---------|
| Read | Yes | Yes | Yes | Yes |
| Write | Yes | No | No | Yes |
| No-std | Yes | No | No | No |
| El-Torito | Yes | No | No | Yes |
| Rock Ridge | Yes | Yes | Partial | Yes |
| Joliet | Yes | Yes | Yes | Yes |
| Language | Rust | Rust | Rust | C |

## Examples

Run the examples with:

```bash
# Read an ISO and display its contents
cargo run --example read_iso -- path/to/image.iso

# Extract files from an ISO
cargo run --example extract_files -- path/to/image.iso ./output

# Create a bootable ISO
cargo run --example create_bootable_iso
```

## Compatibility

ISOs created with this crate are compatible with:
- Linux (`mount`, `isoinfo`, `xorriso`)
- Windows (built-in ISO support)
- macOS (built-in ISO support)
- QEMU/VirtualBox (bootable ISOs)

## Specification References

- ECMA-119 (ISO 9660)
- IEEE P1281 (System Use Sharing Protocol / SUSP)
- IEEE P1282 (Rock Ridge Interchange Protocol / RRIP)
- Joliet Specification (Microsoft)
- El-Torito Bootable CD-ROM Format Specification

## License

This project is licensed under the [MIT license](LICENSE-MIT).

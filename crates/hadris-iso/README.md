# Hadris ISO

A comprehensive Rust implementation of the ISO 9660 filesystem with support for Joliet, Rock Ridge (RRIP), SUSP, El-Torito booting, and no-std environments.

## Features

- **Read & Write Support** - Full-featured ISO creation and extraction
- **No-std Compatible** - Use in bootloaders and custom kernels
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
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
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
let format_options = FormatOptions {
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
    },
};

let mut buffer = Cursor::new(vec![0u8; 1024 * 1024]);
IsoImageWriter::format_new(&mut buffer, files, format_options)?;
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Minimal read support (no-std, no-alloc) | None |
| `alloc` | Heap allocation without full std | `alloc` crate |
| `std` | Full standard library support | `std`, `alloc` |
| `write` | ISO creation/formatting | `std`, `alloc` |
| `joliet` | UTF-16 Unicode filename support | `alloc` |

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-iso = { version = "0.2", default-features = false, features = ["read"] }
```

### For Kernels with Heap (no-std + alloc)

```toml
[dependencies]
hadris-iso = { version = "0.2", default-features = false, features = ["read", "alloc"] }
```

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-iso = { version = "0.2" }  # Uses default features
```

## Extension Support

| Extension | Read | Write | Notes |
|-----------|------|-------|-------|
| ISO 9660 Level 1-3 | Yes | Yes | 8.3 to 31 character filenames, proper truncation handling |
| ISO 9660:1999 | Yes | Yes | Long filenames up to 207 chars (Level 2/3 compliance) |
| SUSP | Yes | Yes | System Use Sharing Protocol for extension framework |
| Joliet | Yes | Yes | UTF-16 BE, up to 64 characters |
| Rock Ridge (RRIP) | Yes | Yes | POSIX semantics, symlinks, uses SUSP |
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

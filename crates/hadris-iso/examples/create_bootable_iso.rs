//! Example: Creating a Bootable ISO
//!
//! This example demonstrates how to create a bootable ISO image with El-Torito
//! boot support. The resulting ISO can be used to boot virtual machines or
//! be written to physical media.
//!
//! Run with: `cargo run --example create_bootable_iso`

use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::boot::EmulationType;
use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};

fn main() {
    // Create a simple boot image (normally this would be a real bootloader)
    // This is just a placeholder that does an infinite loop (jmp $)
    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB; // jmp
    boot_image[1] = 0xFE; // -2 (infinite loop)

    // Create some additional files for the ISO
    let readme_content = b"This is a bootable ISO created with hadris-iso!\n";

    // Prepare the file tree
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![
            IsoFile::File {
                name: Arc::new("boot.bin".to_string()),
                contents: boot_image,
            },
            IsoFile::File {
                name: Arc::new("README.TXT".to_string()),
                contents: readme_content.to_vec(),
            },
            IsoFile::Directory {
                name: Arc::new("docs".to_string()),
                children: vec![IsoFile::File {
                    name: Arc::new("MANUAL.TXT".to_string()),
                    contents: b"User manual goes here.\n".to_vec(),
                }],
            },
        ],
    };

    // Configure El-Torito boot options
    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            // Load 4 sectors (4 * 512 = 2048 bytes)
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            // Enable boot info table if your bootloader needs it
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    // Configure ISO creation options
    let format_options = FormatOptions {
        volume_name: "BOOTABLE_ISO".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: None,
        },
    };

    // Create the ISO in memory
    let mut buffer = Cursor::new(vec![0u8; 512 * 1024]); // 512KB buffer
    IsoImageWriter::format_new(&mut buffer, files, format_options).expect("Failed to create ISO");

    // Write to file
    let iso_data = buffer.into_inner();
    std::fs::write("bootable.iso", &iso_data).expect("Failed to write ISO file");

    println!("Created bootable.iso ({} bytes)", iso_data.len());
    println!();
    println!("You can test it with QEMU:");
    println!("  qemu-system-x86_64 -cdrom bootable.iso -boot d");
}

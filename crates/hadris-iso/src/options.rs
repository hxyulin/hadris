use std::ops::Range;

use bitflags::bitflags;

use crate::FileInput;
#[cfg(feature = "el-torito")]
use crate::boot::EmulationType;

bitflags! {
    /// The extra partition options that the image can have
    #[derive(Debug, Clone, Copy)]
    pub struct PartitionOptions: u8 {
        const PROTECTIVE_MBR = 0b00000001;
    }
}

/// The options for formatting a new ISO image
/// Currently, all the images must be provided in this structure
#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub files: FileInput,
    pub format: PartitionOptions,
    #[cfg(feature = "el-torito")]
    pub boot: Option<BootOptions>,
}

fn align_to_sector(size: usize) -> usize {
    (size + 2047) & !2047
}

fn to_sectors_ceil(size: usize) -> usize {
    (size + 2047) / 2048
}

impl FormatOptions {
    pub const fn new() -> Self {
        FormatOptions {
            files: FileInput::empty(),
            format: PartitionOptions::empty(),
            #[cfg(feature = "el-torito")]
            boot: None,
        }
    }

    pub fn with_files(mut self, files: FileInput) -> Self {
        self.files = files;
        self
    }

    pub fn with_format_options(mut self, options: PartitionOptions) -> Self {
        self.format = options;
        self
    }

    #[cfg(feature = "el-torito")]
    pub fn with_boot_options(mut self, options: BootOptions) -> Self {
        self.boot = Some(options);
        self
    }

    pub fn check(&self) -> Result<(), &'static str> {
        if self.files.len() == 0 {
            return Err("No files provided");
        }

        #[cfg(feature = "el-torito")]
        if let Some(boot) = &self.boot {
            if boot.default.boot_image_path.is_empty() {
                return Err("Default boot image path is empty");
            }
        }

        Ok(())
    }

    /// Calculates the minimum and maximum size of the image
    pub fn image_len(&self) -> (u64, u64) {
        let mut min: u64 = 16 * 2048;
        let mut max: u64 = 16 * 2048;

        let mut path_table_size = 0;

        for file in &self.files {
            if file.is_directory() {
                min += 2048;
                max += 2048;
                path_table_size += (8 + file.path.len() + 1) & !1;
            } else {
                // We are conservative, and we add the minimum
                min += 34;
                // We assume every file is very large
                // TODO: We need to change this dynamically based on enabled extensions
                max += 2048;

                let size = align_to_sector(file.get_data().len()) as u64;
                min += size;
                max += size;
            }
        }

        // We align it and multiply by 2 because we need to store both the
        // little endian and big endian version
        let path_table_size = (align_to_sector(path_table_size) * 2) as u64;
        min += path_table_size;
        max += path_table_size;

        #[cfg(feature = "el-torito")]
        if let Some(boot) = &self.boot {
            // Boot Record Volume Descriptor
            min += 2048;
            max += 2048;

            // Catalog size
            // We add 64 because of the validation entry and default entry
            // The minimum size for a section is 64 bytes (header + 1 entry)
            // The maximum size can technically be more, but we just add 512 for now
            let min_catalog_size = align_to_sector(boot.entries.len() * 64 + 64) as u64;
            let max_catalog_size = align_to_sector(boot.entries.len() * 512 + 64) as u64;
            min += min_catalog_size;
            max += max_catalog_size;

            // Additional size if we write the boot catalogue
            // We need to write the data and the file entry
            if boot.write_boot_catalogue {
                min += min_catalog_size + 34;
                max += max_catalog_size + 2048;
            }
        }

        // TODO: Minimum size is not correct, can be smaller
        (min, max)
    }
}

/// Options for El Torito supported ISO images
#[cfg(feature = "el-torito")]
#[derive(Debug, Clone)]
pub struct BootOptions {
    /// Whether to write the boot catalogue to a boot.catalog file
    pub write_boot_catalogue: bool,

    pub default: BootEntryOptions,
    pub entries: Vec<BootEntryOptions>,
}

#[derive(Debug, Clone)]
#[cfg(feature = "el-torito")]
pub struct BootEntryOptions {
    /// The amount of sectors to load
    pub load_size: u16,
    // The path to the boot image,
    // Currently on root directory is supported
    pub boot_image_path: String,

    /// Whether to write the boot info table, for bootloaders like:
    /// GRUB, LIMINE, SYSLINUX
    pub boot_info_table: bool,

    /// Whether to write the GRUB2 boot info table
    pub grub2_boot_info: bool,

    /// What type of emulation to use
    /// see [`EmulationType`]
    pub emulation: EmulationType,
}

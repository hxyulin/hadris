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
}

/// Options for El Torito supported ISO images
#[cfg(feature = "el-torito")]
#[derive(Debug, Clone)]
pub struct BootOptions {
    /// Whether to write the boot catalogue to a boot.catalog file
    pub write_boot_catalogue: bool,

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
    ///
    pub emulation: EmulationType,
}

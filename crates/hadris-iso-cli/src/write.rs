use std::{fs::OpenOptions, io::Seek, num::NonZeroU16};

use hadris_iso::{
    boot::{
        EmulationType,
        options::{BootEntryOptions, BootOptions, BootSectionOptions},
    },
    joliet::JolietLevel,
    read::PathSeparator,
    write::{
        InputFiles, IsoImageWriter,
        options::{CreationFeatures, FormatOptions},
    },
};
use tracing::Level;

use crate::args::WriteArgs;

pub fn write(args: WriteArgs) {
    let WriteArgs {
        isoroot,
        output,
        verbose,
        level,
        extensions,
        boot,
    } = args;

    let subscriber = tracing_subscriber::fmt::SubscriberBuilder::default()
        .with_max_level(if verbose { Level::TRACE } else { Level::INFO })
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to set subscriber");

    let mut file = OpenOptions::new()
        .truncate(true)
        .read(true)
        .write(true)
        .create(true)
        .open(output)
        .unwrap();
    file.set_len(10_000_000).unwrap();
    let input = InputFiles::from_fs(&isoroot, PathSeparator::ForwardSlash).unwrap();
    let mut ops = FormatOptions {
        volume_name: "TESTISO".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: level.0,
            long_filenames: extensions.level3,
            joliet: if extensions.joliet {
                Some(JolietLevel::Level3)
            } else {
                None
            },
            ..Default::default()
        },
    };
    ops.features.el_torito = Some(BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            load_size: NonZeroU16::new(4),
            boot_image_path: "limine-bios-cd.bin".to_string(),
            boot_info_table: true,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![(
            BootSectionOptions {
                platform: hadris_iso::boot::PlatformId::UEFI,
            },
            BootEntryOptions {
                load_size: None,
                boot_image_path: "limine-uefi-cd.bin".to_string(),
                boot_info_table: false,
                grub2_boot_info: false,
                emulation: EmulationType::NoEmulation,
            },
        )],
    });
    if let Err(err) = IsoImageWriter::format_new(&mut file, input, ops) {
        eprintln!("error occured at: {:?} - {}", file.stream_position(), err);
    }
}

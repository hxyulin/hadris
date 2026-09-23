use std::io::Cursor;
use std::sync::Arc;

use hadris_io::StdIo;
use hadris_iso::boot::EmulationType;
use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::write::options::{
    BaseIsoLevel, CreationFeatures, HybridBootOptions, IsoFormatOptions,
};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter, estimator};
use hadris_part::gpt::types;
use hadris_part::{Gpt, PartitionKind, PartitionTable};
use hadris_storage::{BlockSize, MemDevice};

const BACKUP_GPT_SECTORS: u64 = 33;

fn efi_image_content() -> Vec<u8> {
    (0..3000u32).map(|i| (i % 251) as u8).collect()
}

fn input_files() -> InputFiles {
    let mut bios_image = vec![0u8; 2048];
    bios_image[0] = 0xEB;
    bios_image[1] = 0xFE;

    InputFiles {
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        files: vec![
            IsoFile::File {
                name: Arc::new("boot.bin".to_string()),
                contents: bios_image,
            },
            IsoFile::File {
                name: Arc::new("efi-boot.img".to_string()),
                contents: efi_image_content(),
            },
        ],
    }
}

fn format_options(hybrid_boot: HybridBootOptions) -> IsoFormatOptions {
    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![(
            BootSectionOptions {
                platform: hadris_iso::boot::PlatformId::UEFI,
            },
            BootEntryOptions {
                boot_image_path: "efi-boot.img".to_string(),
                load_size: None,
                boot_info_table: false,
                grub2_boot_info: false,
                emulation: EmulationType::NoEmulation,
            },
        )],
    };

    IsoFormatOptions {
        volume_name: "GPT_ESP_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: Some(hybrid_boot),
        },
        strict_charset: false,
    }
}

fn build_iso(
    hybrid_boot: HybridBootOptions,
) -> Result<Vec<u8>, hadris_iso::write::IsoCreationError> {
    let buffer = IsoImageWriter::create(
        StdIo::new(Cursor::new(Vec::new())),
        input_files(),
        format_options(hybrid_boot),
    )?;
    Ok(buffer.into_inner().into_inner())
}

fn iso_space_sectors_512(iso: &[u8]) -> u64 {
    let pvd = 16 * 2048;
    let volume_space = u32::from_le_bytes(iso[pvd + 80..pvd + 84].try_into().unwrap()) as u64;
    volume_space * 4
}

fn read_gpt(iso: &[u8]) -> Gpt {
    let mut dev = MemDevice::new(iso.to_vec(), BlockSize::new(512).unwrap());
    let disk = hadris_part::sync::read(&mut dev).expect("failed to read back GPT disk");
    match disk.into_table() {
        PartitionTable::Gpt(gpt) => gpt,
        PartitionTable::Hybrid(hybrid) => hybrid.into_gpt(),
        other => panic!("expected a GPT, got {other:?}"),
    }
}

fn check_gpt_with_esp(iso: &[u8]) -> (u64, u64) {
    let total_512 = iso.len() as u64 / 512;

    assert_eq!(
        iso.len() as u64 % 2048,
        0,
        "image length not sector aligned"
    );
    assert_eq!(
        iso_space_sectors_512(iso),
        total_512,
        "volume_space_size does not cover the appended backup GPT region"
    );
    let iso_512 = total_512 - BACKUP_GPT_SECTORS.div_ceil(4) * 4;
    assert_eq!(
        (iso_512 + BACKUP_GPT_SECTORS).div_ceil(4) * 4,
        total_512,
        "unexpected appended region size"
    );

    let gpt = read_gpt(iso);
    assert_eq!(gpt.damaged_copy(), None, "a GPT copy failed validation");
    assert_eq!(gpt.entry_count(), 128);
    assert_eq!(gpt.backup_lba(), total_512 - 1);

    let efi_content = efi_image_content();
    let esp = gpt
        .partitions()
        .find(|p| p.kind() == PartitionKind::Gpt(types::EFI_SYSTEM))
        .expect("no EFI System Partition in GPT");
    let esp_start = esp.start();
    let esp_sectors = esp.len();
    assert_eq!(
        esp_start % 4,
        0,
        "ESP start is not aligned to an ISO sector extent"
    );
    assert_eq!(esp_sectors, (efi_content.len() as u64).div_ceil(512));
    let esp_offset = (esp_start * 512) as usize;
    assert_eq!(
        &iso[esp_offset..esp_offset + efi_content.len()],
        efi_content.as_slice(),
        "ESP extent does not cover the El Torito UEFI image"
    );

    let data = gpt
        .partitions()
        .find(|p| p.kind() == PartitionKind::Gpt(types::BASIC_DATA))
        .expect("no basic data partition in GPT");
    assert_eq!(data.start(), 64);
    assert_eq!(data.end(), esp_start);
    assert_eq!(data.name().unwrap().to_string(), "ISO9660");

    if let Some(tail) = gpt
        .partitions()
        .find(|p| p.kind() == PartitionKind::Gpt(types::BASIC_DATA) && p.start() > esp_start)
    {
        assert_eq!(tail.start(), esp_start + esp_sectors);
        assert_eq!(tail.end(), iso_512);
    }

    (esp_start, esp_sectors)
}

#[derive(Debug, Clone, Copy)]
struct MbrEntry {
    boot: u8,
    part_type: u8,
    start_lba: u32,
    sector_count: u32,
}

fn parse_mbr(iso: &[u8]) -> Vec<MbrEntry> {
    assert_eq!(iso[510], 0x55);
    assert_eq!(iso[511], 0xAA);
    (0..4)
        .map(|i| {
            let base = 446 + i * 16;
            MbrEntry {
                boot: iso[base],
                part_type: iso[base + 4],
                start_lba: u32::from_le_bytes(iso[base + 8..base + 12].try_into().unwrap()),
                sector_count: u32::from_le_bytes(iso[base + 12..base + 16].try_into().unwrap()),
            }
        })
        .filter(|e| e.part_type != 0)
        .collect()
}

#[test]
fn estimate_covers_appended_backup_gpt() {
    for opts in [HybridBootOptions::gpt(), HybridBootOptions::hybrid()] {
        let est = estimator::estimate(&input_files(), &format_options(opts.clone()));
        let iso = build_iso(opts).expect("failed to create ISO");
        assert_eq!(est.breakdown.backup_gpt, 36 * 512);
        assert!(
            est.minimum_bytes() >= iso.len() as u64,
            "estimate {} smaller than actual image {}",
            est.minimum_bytes(),
            iso.len()
        );
    }
}

#[test]
fn gpt_backup_and_esp_with_explicit_option() {
    let iso = build_iso(HybridBootOptions::gpt().with_efi_boot_partition("efi-boot.img"))
        .expect("failed to create GPT ISO");
    check_gpt_with_esp(&iso);
}

#[test]
fn gpt_esp_autodetected_from_el_torito() {
    let options = HybridBootOptions::gpt();
    assert!(options.efi_boot_partition.is_none());
    let iso = build_iso(options).expect("failed to create GPT ISO");
    check_gpt_with_esp(&iso);
}

#[test]
fn gpt_missing_efi_boot_partition_fails() {
    let result = build_iso(HybridBootOptions::gpt().with_efi_boot_partition("missing.img"));
    assert!(result.is_err(), "expected missing ESP image to fail");
}

#[test]
fn hybrid_backup_esp_and_mbr_mirror() {
    let iso = build_iso(HybridBootOptions::hybrid()).expect("failed to create hybrid ISO");
    let (esp_start, esp_sectors) = check_gpt_with_esp(&iso);

    let entries = parse_mbr(&iso);
    let protective = entries
        .iter()
        .find(|e| e.part_type == 0xEE)
        .expect("no protective MBR entry");
    assert_eq!(protective.start_lba, 1);
    assert_eq!(protective.sector_count, 63);

    let iso_part = entries
        .iter()
        .find(|e| e.part_type == 0x17)
        .expect("no mirrored ISO9660 MBR entry");
    assert_eq!(iso_part.start_lba, 64);
    assert_eq!(iso_part.boot, 0x80);

    let esp = entries
        .iter()
        .find(|e| e.part_type == 0xEF)
        .expect("no mirrored EFI MBR entry");
    assert_eq!(u64::from(esp.start_lba), esp_start);
    assert_eq!(u64::from(esp.sector_count), esp_sectors);
    assert_eq!(esp.boot, 0x00);
}

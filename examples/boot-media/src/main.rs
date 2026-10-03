//! Builds the boot media of a small operating system, then reads every image
//! back and checks it:
//!
//! - `usb.img`: a GPT disk whose EFI system partition is a FAT volume built
//!   from a tree, with the partition start recorded as hidden sectors.
//! - `boot.iso`: an ISO 9660 image that boots BIOS through El Torito and UEFI
//!   through one ESP shared by El Torito and a hybrid GPT.
//! - `bridge.iso`: the same files as an ISO 9660 and UDF bridge.
//!
//! Catalog actions: IO-PART-01, IO-DETECT-01, IO-DETECT-02, VOL-FORMAT-03,
//! BUILD-ESP-01, BUILD-TREE-01, BOOT-ET-WRITE-01, BOOT-ET-READ-01,
//! BOOT-HYB-01, BUILD-BRIDGE-01.
//!
//! ```text
//! cargo run -p hadris-example-boot-media            # build and check in memory
//! cargo run -p hadris-example-boot-media -- out/    # also write the images
//! ```

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use hadris::ImageFormat;
use hadris::fat::FatOptions;
use hadris::fs::sync::Volume;
use hadris::fs::{Content, MountOptions, Node, OpenOptions, Tree};
use hadris::iso::sync::IsoFs;
use hadris::iso::{
    AppendedPartition, BootEntry, BootInfo, ElTorito, Hybrid, IsoId, IsoOptions, Platform,
};
use hadris::part::gpt::types;
use hadris::part::{DiskLayout, Guid, PartitionKind, PartitionSpec, Size};
use hadris::storage::{BlockSize, MemDevice};
use hadris::sync::AnyFs;
use hadris::udf::UdfOptions;

const LOADER: &[u8] = b"stand-in for a PE32+ EFI application";
const KERNEL: &[u8] = b"stand-in for a kernel image";
const ESP_SIZE: u64 = 8 << 20;
const SECTOR: BlockSize = BlockSize::new(512).unwrap();
const ISO_BLOCK: BlockSize = BlockSize::new(2048).unwrap();

fn main() -> Result<()> {
    let out = std::env::args_os().nth(1).map(PathBuf::from);

    let usb = usb_disk()?;
    check_usb(&usb)?;
    let esp = esp_image()?;
    let iso = boot_iso(&esp)?;
    check_boot_iso(&iso, &esp)?;
    let bridge = bridge_image()?;
    check_bridge(&bridge)?;

    if let Some(dir) = out {
        std::fs::create_dir_all(&dir)?;
        for (name, bytes) in [
            ("usb.img", &usb),
            ("boot.iso", &iso),
            ("bridge.iso", &bridge),
        ] {
            std::fs::write(dir.join(name), bytes)?;
            println!("wrote {}", dir.join(name).display());
        }
    }
    println!("boot media built and checked");
    Ok(())
}

fn esp_tree() -> Result<Tree> {
    let mut tree = Tree::new();
    tree.insert("EFI/BOOT/BOOTX64.EFI", Node::file(Content::bytes(LOADER)))?;
    tree.insert("kernel.elf", Node::file(Content::bytes(KERNEL)))?;
    Ok(tree)
}

/// A standalone ESP image, as El Torito and an appended partition store it.
fn esp_image() -> Result<Vec<u8>> {
    let mut image = Vec::new();
    hadris::fat::sync::write(
        &mut image,
        &esp_tree()?,
        &FatOptions::new().with_size(ESP_SIZE),
    )?;
    Ok(image)
}

/// A GPT disk with an ESP and a root partition. The FAT writer sees only the
/// ESP, through the window `part::sync::open` returns.
fn usb_disk() -> Result<Vec<u8>> {
    let mut disk = MemDevice::new(vec![0u8; 64 << 20], SECTOR);
    let layout = DiskLayout::gpt(Guid::from_bytes(*b"hadris-boot-disk"))
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(32)).with_name("EFI system"))
        .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining).with_name("root"));
    let table = hadris::part::sync::create(&mut disk, &layout)?;
    let esp = table.partition(0).context("the layout has no ESP")?;
    let window = hadris::part::sync::open(&mut disk, &esp)?;
    hadris::fat::sync::write(window, &esp_tree()?, &FatOptions::new())?;
    Ok(disk.into_inner())
}

fn check_usb(bytes: &[u8]) -> Result<()> {
    let mut disk = MemDevice::new(bytes, SECTOR);
    let found = hadris::sync::detect(&mut disk)?;
    ensure!(
        found.first().map(|c| c.format()) == Some(ImageFormat::Gpt),
        "usb.img should detect as GPT"
    );
    let table = hadris::part::sync::read(&mut disk)?;
    let esp = table
        .partitions()
        .find(|p| p.kind() == PartitionKind::Gpt(types::EFI_SYSTEM))
        .context("usb.img has no ESP")?;
    let window = hadris::part::sync::open(&mut disk, &esp)?;
    let AnyFs::Fat(mut fat) = hadris::sync::open(window, MountOptions::new().read_only())
        .map_err(|err| err.into_error())?
    else {
        bail!("the ESP should mount as FAT");
    };

    let mut hidden = [0u8; 4];
    fat.read_raw(28, &mut hidden)?;
    ensure!(
        u64::from(u32::from_le_bytes(hidden)) == esp.start(),
        "the boot sector should record the partition start as hidden sectors"
    );
    ensure!(read_file(Volume::new(fat), "/EFI/BOOT/BOOTX64.EFI")? == LOADER);
    Ok(())
}

fn boot_iso(esp: &[u8]) -> Result<Vec<u8>> {
    let mut tree = Tree::new();
    tree.insert(
        "boot/bios.img",
        Node::file(Content::bytes(vec![0xEB; 2048])),
    )?;
    tree.insert("kernel.elf", Node::file(Content::bytes(KERNEL)))?;
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "HADRIS_OS")
        .with_joliet()
        .with_rock_ridge()
        .with_el_torito(
            ElTorito::new()
                .with_entry(
                    BootEntry::bios("boot/bios.img")
                        .with_load_size(4)
                        .with_boot_info(BootInfo::Table),
                )
                .with_entry(BootEntry::uefi_appended(0)),
        )
        .with_hybrid(
            Hybrid::gpt_hybrid_mbr()
                .with_appended(AppendedPartition::esp(Content::bytes(esp.to_vec()))),
        );
    let mut image = Vec::new();
    let report = hadris::iso::sync::write(&mut image, &tree, &options)?;
    for warning in report.warnings() {
        println!("boot.iso: {warning}");
    }
    Ok(image)
}

fn check_boot_iso(bytes: &[u8], esp: &[u8]) -> Result<()> {
    let mut disk = MemDevice::new(bytes, SECTOR);
    let formats: Vec<_> = hadris::sync::detect(&mut disk)?
        .iter()
        .map(|c| c.format())
        .collect();
    ensure!(
        formats.first() == Some(&ImageFormat::Iso) && formats.contains(&ImageFormat::Gpt),
        "boot.iso should detect as ISO 9660 with a GPT, got {formats:?}"
    );

    let mut iso = IsoFs::mount(MemDevice::new(bytes, ISO_BLOCK), MountOptions::new())
        .map_err(|err| err.into_error())?;
    let mut buf = [0u8; 2048];
    let catalog = iso
        .boot_catalog(&mut buf)?
        .context("boot.iso has no boot catalog")?;
    let entries: Vec<_> = catalog.entries().collect();
    ensure!(entries.len() == 2, "expected a BIOS and a UEFI entry");
    ensure!(entries[0].platform() == Platform::X86 && entries[0].is_bootable());
    ensure!(entries[1].platform() == Platform::Efi && entries[1].is_bootable());
    let start = u64::from(entries[1].load_block()) * u64::from(ISO_BLOCK.get());
    ensure!(
        bytes.get(start as usize..start as usize + esp.len()) == Some(esp),
        "the UEFI entry should point at the ESP"
    );
    ensure!(read_file(Volume::new(iso), "/kernel.elf")? == KERNEL);

    let table = hadris::part::sync::read(&mut disk)?;
    let part = table
        .partitions()
        .find(|p| p.kind() == PartitionKind::Gpt(types::EFI_SYSTEM))
        .context("boot.iso has no GPT ESP")?;
    ensure!(
        part.start() * u64::from(SECTOR.get()) == start,
        "El Torito and the GPT should share one copy of the ESP"
    );
    let window = hadris::part::sync::open(&mut disk, &part)?;
    let fat = hadris::sync::open(window, MountOptions::new().read_only())
        .map_err(|err| err.into_error())?;
    ensure!(read_file(Volume::new(fat), "/EFI/BOOT/BOOTX64.EFI")? == LOADER);
    Ok(())
}

fn bridge_image() -> Result<Vec<u8>> {
    let mut tree = esp_tree()?;
    tree.insert("README.txt", Node::file(Content::bytes("ISO and UDF\n")))?;
    let mut image = Vec::new();
    hadris::udf::sync::write_bridge(
        &mut image,
        &tree,
        &IsoOptions::default().with_joliet(),
        &UdfOptions::default(),
    )?;
    Ok(image)
}

fn check_bridge(bytes: &[u8]) -> Result<()> {
    let mut disk = MemDevice::new(bytes, ISO_BLOCK);
    let found = hadris::sync::detect(&mut disk)?;
    ensure!(found.first().map(|c| c.format()) == Some(ImageFormat::IsoUdfBridge));
    let AnyFs::Udf(udf) =
        hadris::sync::open(&mut disk, MountOptions::new()).map_err(|err| err.into_error())?
    else {
        bail!("a bridge should open as UDF");
    };
    ensure!(read_file(Volume::new(udf), "/README.txt")? == b"ISO and UDF\n");
    let iso = IsoFs::mount(&mut disk, MountOptions::new()).map_err(|err| err.into_error())?;
    ensure!(read_file(Volume::new(iso), "/README.txt")? == b"ISO and UDF\n");
    Ok(())
}

fn read_file<F>(vol: Volume<F>, path: &str) -> Result<Vec<u8>>
where
    F: hadris::fs::sync::FileSystem,
    F::DeviceError: std::error::Error + Send + Sync + 'static,
{
    let mut data = Vec::new();
    vol.open(path, OpenOptions::new().read())
        .with_context(|| format!("failed to open {path}"))?
        .read_to_end(&mut data)?;
    Ok(data)
}

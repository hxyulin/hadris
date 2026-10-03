//! Forensic inspection: examines evidence images without changing a byte.
//!
//! - A FAT32 image is opened through a device that refuses writes, as behind
//!   a write blocker, and mounts read-only on its own.
//! - Its geometry, raw boot sector and the clusters of a file are read, and
//!   the file is carved back from the raw extents.
//! - A read-only mount of a writable copy is synced and unmounted, and the
//!   copy is still identical.
//! - A copy with a damaged boot sector is reported as damaged FAT rather
//!   than as unknown, and still mounts from the backup boot sector.
//! - The offline checker reports a damaged backup boot sector.
//! - An ISO 9660 image's identifiers, descriptors and Rock Ridge
//!   ownership are read.
//!
//! Catalog actions: IO-RO-01, IO-DETECT-01, VOL-MOUNT-02, VOL-MOUNT-06,
//! VOL-INFO-01, VOL-INFO-02, VOL-SERIAL-01, BOOT-VBR-03, META-RAW-01,
//! META-OWNER-01, FILE-EXTENT-01, CHECK-FSCK-01.
//!
//! ```text
//! cargo run -p hadris-example-inspect
//! ```

use std::io::Read;

use anyhow::{Context, Result, ensure};
use hadris::fat::sync::FatFs;
use hadris::fat::{FatKind, FatOptions};
use hadris::fs::sync::{FileSystem, Volume};
use hadris::fs::{Content, Extent, MountOptions, Node, OpenOptions, Owner, Resolve, SetAttr, Tree};
use hadris::iso::raw::VolumeDescriptor;
use hadris::iso::sync::IsoFs;
use hadris::iso::{IsoId, IsoOptions};
use hadris::storage::{BlockSize, MemDevice};
use hadris::{ErrorKind, ImageFormat};

const SECTOR: BlockSize = BlockSize::new(512).unwrap();

fn main() -> Result<()> {
    let report: Vec<u8> = (0..20_000u32)
        .map(|i| b"evidence "[i as usize % 9])
        .collect();
    let mut tree = Tree::new();
    tree.insert(
        "case/report.txt",
        Node::file(Content::bytes(report.clone())),
    )?;
    tree.insert("case/notes.txt", Node::file(Content::bytes("seen\n")))?;
    let options = FatOptions::new()
        .with_kind(FatKind::Fat32)
        .with_size(64 << 20)
        .with_serial(0x1234_ABCD);
    let mut image = Vec::new();
    hadris::fat::sync::write(&mut image, &tree, &options)?;

    write_blocked(&image, &report)?;
    read_only_mount_changes_nothing(&image)?;
    damaged_boot_sector(&image, &report)?;
    checker_reports_damage(&image)?;
    iso_identifiers()?;

    println!("evidence inspected; no image changed");
    Ok(())
}

fn write_blocked(image: &[u8], report: &[u8]) -> Result<()> {
    let mut blocked = MemDevice::new(image, SECTOR);
    let found = hadris::sync::detect(&mut blocked)?;
    let first = found.first().context("nothing detected")?;
    ensure!(first.format() == ImageFormat::Fat(FatKind::Fat32) && first.damage().is_none());

    let mut fs = FatFs::mount(blocked, MountOptions::new()).map_err(|err| err.into_error())?;
    ensure!(
        fs.is_read_only(),
        "a device that refuses writes mounts read-only"
    );
    let info = *fs.info();
    ensure!(info.kind() == FatKind::Fat32 && info.fat_count() == 2);
    println!(
        "FAT32: {} clusters of {} bytes, data at byte {}",
        info.max_cluster() - 1,
        info.cluster_size(),
        info.data_start()
    );
    ensure!(info.volume_serial() == Some(0x1234_ABCD));

    let mut boot = [0u8; 512];
    fs.read_raw(0, &mut boot)?;
    ensure!(boot[510..] == [0x55, 0xAA] && &boot[82..90] == b"FAT32   ");

    let node = fs.resolve(b"/case/report.txt", Resolve::Lexical)?;
    let mut carved = Vec::new();
    let mut runs = [Extent::new(0, 0); 4];
    loop {
        let n = fs.extents(node, carved.len() as u64, &mut runs)?;
        if n == 0 {
            break;
        }
        for run in &runs[..n] {
            let mut bytes = vec![0; usize::try_from(run.len())?];
            fs.read_raw(run.offset(), &mut bytes)?;
            carved.extend_from_slice(&bytes);
        }
    }
    ensure!(carved == report, "the extents should hold the file's bytes");
    let mut entry = [Extent::new(0, 0); 1];
    ensure!(fs.records(node, &mut entry)? == 1);
    let mut short = [0u8; 32];
    fs.read_raw(entry[0].offset(), &mut short)?;
    ensure!(&short[..11] == b"REPORT  TXT");
    fs.forget(node, 1);

    let vol = Volume::new(fs);
    let refused = vol
        .open("/case/notes.txt", OpenOptions::new().write())
        .err()
        .context("opening for writing should fail")?;
    ensure!(refused.kind() == ErrorKind::ReadOnly);
    Ok(())
}

fn read_only_mount_changes_nothing(image: &[u8]) -> Result<()> {
    let copy = MemDevice::new(image.to_vec(), SECTOR);
    let fs = FatFs::mount(copy, MountOptions::new().read_only()).map_err(|err| err.into_error())?;
    let vol = Volume::new(fs);
    let mut data = Vec::new();
    vol.open("/case/notes.txt", OpenOptions::new().read())?
        .read_to_end(&mut data)?;
    for entry in vol.read_dir("/case")? {
        entry?;
    }
    vol.lock().sync()?;
    let fs = vol.into_inner().ok().context("a handle is still open")?;
    let copy = fs.unmount().map_err(|err| err.into_error())?;
    ensure!(
        copy.into_inner() == image,
        "a read-only mount should write nothing"
    );
    Ok(())
}

fn damaged_boot_sector(image: &[u8], report: &[u8]) -> Result<()> {
    let mut damaged = image.to_vec();
    damaged[11..13].copy_from_slice(&[0, 0]);
    let mut dev = MemDevice::new(&damaged[..], SECTOR);
    let found = hadris::sync::detect(&mut dev)?;
    let first = found
        .first()
        .context("the damaged image should still detect")?;
    ensure!(matches!(first.format(), ImageFormat::Fat(_)));
    let damage = first.damage().context("the damage should be reported")?;
    ensure!(damage.kind() == ErrorKind::Corrupt);

    let backup = MountOptions::new().backup_boot();
    let fs = FatFs::mount(dev, backup).map_err(|err| err.into_error())?;
    let mut data = Vec::new();
    Volume::new(fs)
        .open("/case/report.txt", OpenOptions::new().read())?
        .read_to_end(&mut data)?;
    ensure!(data == report);
    Ok(())
}

fn checker_reports_damage(image: &[u8]) -> Result<()> {
    let mut damaged = image.to_vec();
    damaged[6 * 512 + 3] ^= 0xFF;
    let mut dev = MemDevice::new(&damaged[..], SECTOR);
    let mut findings = Vec::new();
    let report = hadris::fat::sync::check(&mut dev, &mut [0u8; 4096], |finding| {
        findings.push(finding.message().to_owned());
    })?;
    ensure!(!report.is_clean(), "the backup boot sector differs");
    println!("check: {findings:?}");
    Ok(())
}

fn iso_identifiers() -> Result<()> {
    let mut tree = Tree::new();
    tree.insert(
        "owned.txt",
        Node::file(Content::bytes("x")).with_attrs(SetAttr::new().with_owner(Owner::new(1000, 50))),
    )?;
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "EVIDENCE_01")
        .with_id(IsoId::Publisher, "HADRIS LAB")
        .with_rock_ridge()
        .with_joliet();
    let mut image = Vec::new();
    hadris::iso::sync::write(&mut image, &tree, &options)?;

    let mut iso = IsoFs::mount(
        MemDevice::new(&image[..], BlockSize::new(2048).unwrap()),
        MountOptions::new(),
    )
    .map_err(|err| err.into_error())?;
    ensure!(iso.info().id(IsoId::Volume) == b"EVIDENCE_01");
    ensure!(iso.info().id(IsoId::Publisher) == b"HADRIS LAB");

    let mut kinds = Vec::new();
    let mut index = 0;
    while let Some(descriptor) = iso.descriptor(index)? {
        kinds.push(match descriptor {
            VolumeDescriptor::Primary(_) => "primary",
            VolumeDescriptor::Supplementary(_) => "supplementary",
            VolumeDescriptor::BootRecord(_) => "boot",
            VolumeDescriptor::Terminator(_) => "terminator",
            _ => "other",
        });
        index += 1;
    }
    ensure!(
        kinds == ["primary", "supplementary", "terminator"],
        "{kinds:?}"
    );

    let node = iso.resolve(b"/owned.txt", Resolve::Lexical)?;
    let rr = iso.rock_ridge(node)?.context("no Rock Ridge entries")?;
    ensure!(rr.owner() == Some((1000, 50)));
    ensure!(iso.stat(node)?.owner() == Some(Owner::new(1000, 50)));
    iso.forget(node, 1);
    Ok(())
}

//! Formats a FAT16 and an exFAT image file, edits each through the same
//! generic code (directories, writes, appends, renames, removals), sets the
//! label, unmounts, checks the result offline and reopens it read-only.
//!
//! Catalog actions: VOL-FORMAT-07, VOL-MOUNT-01,
//! VOL-MOUNT-02, VOL-LABEL-01, VOL-LABEL-02, VOL-STAT-01, VOL-SYNC-01,
//! VOL-UMOUNT-01, DIR-MKDIR-02, DIR-LIST-01, DIR-RMDIR-01, FILE-OPEN-01,
//! FILE-WRITE-01, FILE-WRITE-03, FILE-RENAME-01, FILE-UNLINK-01,
//! CHECK-FSCK-01.
//!
//! ```text
//! cargo run -p hadris-example-image-edit            # in a temporary directory
//! cargo run -p hadris-example-image-edit -- out/    # keep the images
//! ```

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use hadris::ErrorKind;
use hadris::fat::exfat::sync::ExFatFs;
use hadris::fat::sync::FatFs;
use hadris::fat::{FatKind, FatOptions, exfat};
use hadris::fs::sync::{FileSystem, Volume};
use hadris::fs::{MountOptions, OpenOptions, SystemClock};
use hadris::host::FileDevice;

const IMAGE_SIZE: u64 = 64 << 20;
const REPORT: &[u8] = b"first draft\nsecond line\n";

fn main() -> Result<()> {
    let (dir, keep) = match std::env::args_os().nth(1) {
        Some(dir) => (PathBuf::from(dir), true),
        None => (
            std::env::temp_dir().join(format!("hadris-image-edit-{}", std::process::id())),
            false,
        ),
    };
    std::fs::create_dir_all(&dir)?;
    let result = fat(&dir.join("fat16.img")).and_then(|()| exfat(&dir.join("exfat.img")));
    if !keep {
        std::fs::remove_dir_all(&dir)?;
    }
    result?;
    println!("FAT16 and exFAT images edited and checked");
    Ok(())
}

fn fat(path: &Path) -> Result<()> {
    let mut dev = new_image(path)?;
    let options = FatOptions::new()
        .with_kind(FatKind::Fat16)
        .with_label(hadris::fat::VolumeLabel::new("BEFORE")?);
    hadris::fat::sync::format(&mut dev, &options)?;

    let fs = FatFs::mount(dev, MountOptions::new().with_clock(&SystemClock))
        .map_err(|err| err.into_error())?;
    let vol = Volume::new(fs);
    edit(&vol)?;
    vol.lock()
        .set_label(Some(hadris::fat::VolumeLabel::new("AFTER")?))?;
    let fs = vol.into_inner().ok().context("a handle is still open")?;
    let mut dev = fs.unmount().map_err(|err| err.into_error())?;

    let report = hadris::fat::sync::check(&mut dev, &mut [0u8; 4096], |finding| {
        eprintln!("{finding}");
    })?;
    ensure!(
        report.is_clean(),
        "fsck found {} problems",
        report.findings()
    );

    let fs = FatFs::mount(dev, MountOptions::new().read_only()).map_err(|err| err.into_error())?;
    verify(&Volume::new(fs), "AFTER")
}

fn exfat(path: &Path) -> Result<()> {
    let mut dev = new_image(path)?;
    let options = exfat::ExFatOptions::new().with_label(exfat::VolumeLabel::new("Before")?);
    exfat::sync::format(&mut dev, &options)?;

    let fs = ExFatFs::mount(dev, MountOptions::new().with_clock(&SystemClock))
        .map_err(|err| err.into_error())?;
    let vol = Volume::new(fs);
    edit(&vol)?;
    vol.lock()
        .set_label(Some(exfat::VolumeLabel::new("After")?))?;
    let fs = vol.into_inner().ok().context("a handle is still open")?;
    let mut dev = fs.unmount().map_err(|err| err.into_error())?;

    let report = exfat::sync::check(&mut dev, &mut [0u8; 4096], |finding| {
        eprintln!("{finding}");
    })?;
    ensure!(
        report.is_clean(),
        "fsck found {} problems",
        report.findings()
    );

    let fs =
        ExFatFs::mount(dev, MountOptions::new().read_only()).map_err(|err| err.into_error())?;
    verify(&Volume::new(fs), "After")
}

fn new_image(path: &Path) -> Result<FileDevice> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.set_len(IMAGE_SIZE)?;
    Ok(FileDevice::new(file)?)
}

/// The edits a host tool makes, written once for every writable driver.
fn edit<F>(vol: &Volume<F>) -> Result<()>
where
    F: FileSystem,
    F::DeviceError: std::error::Error + Send + Sync + 'static,
{
    let before = vol.lock().statfs()?;

    vol.create_dir_all("/docs/drafts")?;
    let mut file = vol.open(
        "/docs/drafts/report.txt",
        OpenOptions::new().write().create_new(),
    )?;
    file.write_all(b"first draft\n")?;
    file.close()?;
    let mut file = vol.open(
        "/docs/drafts/report.txt",
        OpenOptions::new().write().append(),
    )?;
    file.write_all(b"second line\n")?;
    file.close()?;
    vol.rename("/docs/drafts/report.txt", "/docs/Final Report.txt")?;
    vol.remove_dir("/docs/drafts")?;

    let mut scratch = vol.open("/scratch.tmp", OpenOptions::new().write().create())?;
    scratch.write_all(&vec![0x5A; 256 << 10])?;
    scratch.close()?;
    ensure!(vol.lock().statfs()?.free_blocks() < before.free_blocks());
    vol.remove_file("/scratch.tmp")?;
    let gone = vol.metadata("/scratch.tmp").unwrap_err();
    ensure!(gone.kind() == ErrorKind::NotFound);

    vol.lock().sync()?;
    Ok(())
}

fn verify<F>(vol: &Volume<F>, label: &str) -> Result<()>
where
    F: FileSystem,
    F::DeviceError: std::error::Error + Send + Sync + 'static,
{
    let mut buf = [0u8; 64];
    ensure!(vol.lock().label(&mut buf)? == Some(label));

    let names: Vec<String> = vol
        .read_dir("/docs")?
        .map(|entry| Ok(entry?.name().to_str().unwrap_or("?").to_owned()))
        .collect::<Result<_>>()?;
    ensure!(names == ["Final Report.txt"], "unexpected /docs: {names:?}");

    let mut data = Vec::new();
    vol.open("/docs/Final Report.txt", OpenOptions::new().read())?
        .read_to_end(&mut data)?;
    ensure!(data == REPORT);

    let refused = vol.create_dir("/new").unwrap_err();
    ensure!(refused.kind() == ErrorKind::ReadOnly, "got {refused}");
    Ok(())
}

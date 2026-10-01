//! Firmware on a microcontroller with an SD card, using only the embedded
//! API (no allocator, fixed buffers, a caller-owned `MountToken`):
//!
//! - A data logger appends one record per boot to a FAT16 card that a host
//!   formatted, and the host reads the log back.
//! - The card locks itself in the middle of a session: the write is
//!   refused, the mount turns read-only, unmount reports the unwritten
//!   size and returns the card, and the records already closed survive.
//! - A camera-style exFAT card written on the host is read by the firmware.
//!
//! Runs on the host; `examples/firmware` builds the same calls for
//! bare-metal targets.
//!
//! Catalog actions: IO-OPEN-01, IO-RO-02, VOL-MOUNT-01, VOL-UMOUNT-01,
//! VOL-LABEL-01, VOL-STAT-01, DIR-MKDIR-01, DIR-LIST-01, FILE-WRITE-03,
//! FILE-READ-01, NF-NOALLOC-01, NF-NOALLOC-02.
//!
//! ```text
//! cargo run -p hadris-example-sd-logger
//! ```

use std::cell::Cell;
use std::io::{Read, Write};
use std::ops::ControlFlow;
use std::rc::Rc;

use anyhow::{Context, Result, ensure};
use hadris::ErrorKind;
use hadris::fat::embedded::sync::Fat;
use hadris::fat::embedded::{MountToken, Options};
use hadris::fat::exfat::embedded::sync::ExFat;
use hadris::fat::exfat::sync::ExFatFs;
use hadris::fat::sync::FatFs;
use hadris::fat::{FatKind, FatOptions, exfat};
use hadris::fs::sync::Volume;
use hadris::fs::{DirCursor, MountOptions, OpenOptions};
use hadris::io::{Error, ErrorType};
use hadris::storage::sync::BlockDevice;
use hadris::storage::{BlockIndex, BlockSize, MemDevice};

const SECTOR: BlockSize = BlockSize::new(512).unwrap();

/// An SD card whose write-protect can trip at any moment.
struct Card {
    blocks: MemDevice<Vec<u8>>,
    locked: Rc<Cell<bool>>,
}

impl ErrorType for Card {
    type Error = core::convert::Infallible;
}

impl BlockDevice for Card {
    fn block_size(&self) -> BlockSize {
        self.blocks.block_size()
    }

    fn block_count(&self) -> u64 {
        self.blocks.block_count()
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.blocks.read_blocks(first, buf)
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        if self.locked.get() {
            return Err(Error::new(ErrorKind::ReadOnly, "card is write-protected"));
        }
        self.blocks.write_blocks(first, buf)
    }
}

/// One boot of the logger: appends `record` to `LOGS/DATA.CSV`.
fn log_boot<D: BlockDevice>(card: D, record: &[u8]) -> Result<D, hadris::MountError<D, D::Error>> {
    let mut token = MountToken::new();
    let mut fat: Fat<'_, D> = Fat::mount_with(card, &mut token, Options::new())?;
    let root = fat.root();
    let result = (|| {
        let logs = match fat.open_dir(root, "LOGS") {
            Ok(dir) => dir,
            Err(err) if err.kind() == ErrorKind::NotFound => fat.create_dir(root, "LOGS")?,
            Err(err) => return Err(err),
        };
        let file = fat.open(
            logs,
            "DATA.CSV",
            OpenOptions::new().write().create().append(),
        )?;
        fat.write(&file, record)?;
        fat.close(file)
    })();
    match result {
        Ok(()) => fat.unmount(),
        Err(err) => Err(hadris::MountError::new(err, fat.into_inner())),
    }
}

fn main() -> Result<()> {
    let mut blocks = MemDevice::new(vec![0u8; 32 << 20], SECTOR);
    hadris::fat::sync::format(&mut blocks, &FatOptions::new().with_kind(FatKind::Fat16))?;
    let locked = Rc::new(Cell::new(false));
    let mut card = Card {
        blocks,
        locked: locked.clone(),
    };

    for boot in 1..=3 {
        card =
            log_boot(card, format!("boot,{boot}\n").as_bytes()).map_err(|err| err.into_error())?;
    }

    card = card_locks_mid_session(card, &locked)?;
    locked.set(false);

    let fs = FatFs::mount(card.blocks, MountOptions::new().read_only())
        .map_err(|err| err.into_error())?;
    let mut log = String::new();
    Volume::new(fs)
        .open("/LOGS/DATA.CSV", OpenOptions::new().read())?
        .read_to_string(&mut log)?;
    ensure!(log == "boot,1\nboot,2\nboot,3\n", "log is {log:?}");

    camera_card()?;
    println!("SD logger sessions behaved as firmware expects");
    Ok(())
}

fn card_locks_mid_session(card: Card, locked: &Cell<bool>) -> Result<Card> {
    let mut token = MountToken::new();
    let mut fat: Fat<'_, Card> = Fat::mount(card, &mut token).map_err(|err| err.into_error())?;
    let root = fat.root();
    let logs = fat.open_dir(root, "LOGS")?;
    let file = fat.open(logs, "DATA.CSV", OpenOptions::new().write().append())?;
    fat.write(&file, b"boot,4\n")?;

    locked.set(true);
    let refused = fat.create_dir(root, "NEW").unwrap_err();
    ensure!(refused.kind() == ErrorKind::ReadOnly);
    ensure!(
        fat.is_read_only(),
        "a refused write turns the mount read-only"
    );
    drop(file);
    let err = fat
        .unmount()
        .err()
        .context("unmount should report the size it could not write")?;
    ensure!(err.kind() == ErrorKind::ReadOnly);
    Ok(err.into_device())
}

fn camera_card() -> Result<()> {
    let photo: Vec<u8> = (0..100_000u32).map(|i| (i % 253) as u8).collect();
    let mut blocks = MemDevice::new(vec![0u8; 64 << 20], SECTOR);
    let label = exfat::VolumeLabel::new("CAMERA")?;
    exfat::sync::format(&mut blocks, &exfat::ExFatOptions::new().with_label(label))?;
    let vol =
        Volume::new(ExFatFs::mount(blocks, MountOptions::new()).map_err(|err| err.into_error())?);
    vol.create_dir_all("/DCIM/100HADRS")?;
    let mut file = vol.open(
        "/DCIM/100HADRS/IMG_0001.JPG",
        OpenOptions::new().write().create(),
    )?;
    file.write_all(&photo)?;
    file.close()?;
    let fs = vol.into_inner().ok().context("a handle is still open")?;
    let blocks = fs.unmount().map_err(|err| err.into_error())?;

    let mut token = MountToken::new();
    let mut card: ExFat<'_, _> =
        ExFat::mount(blocks, &mut token).map_err(|err| err.into_error())?;
    let mut name = [0u8; 44];
    ensure!(card.label(&mut name)? == Some("CAMERA"));
    ensure!(card.stats()?.free_blocks() > 0);

    let root = card.root();
    let mut entries = 0;
    card.list(root, DirCursor::START, |_| {
        entries += 1;
        ControlFlow::Continue(())
    })?;
    ensure!(entries == 1, "the root holds DCIM");
    let dcim = card.open_dir(root, "DCIM")?;
    let dir = card.open_dir(dcim, "100HADRS")?;
    let file = card.open(dir, "IMG_0001.JPG", OpenOptions::new().read())?;
    let mut buf = [0u8; 4096];
    let mut offset = 0;
    loop {
        let n = card.read(&file, &mut buf)?;
        if n == 0 {
            break;
        }
        ensure!(buf[..n] == photo[offset..offset + n]);
        offset += n;
    }
    ensure!(offset == photo.len());
    card.close(file)?;
    let write = card.open(dir, "IMG_0001.JPG", OpenOptions::new().write());
    ensure!(write.is_err(), "the embedded exFAT reader does not write");
    card.unmount();
    Ok(())
}

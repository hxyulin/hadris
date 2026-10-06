//! Firmware-shaped sessions on the embedded `Fat` and `ExFat`.
//!
//! Each binary runs one session. On a bare-metal target it is a `no_std`
//! image over [`Card`], a device whose contents the optimizer cannot see,
//! and `scripts/firmware-size.py` reports its flash and worst-case stack
//! (NF-FLASH-01, NF-STACK-01). On the host it runs the same session on a
//! freshly formatted memory device.

#![no_std]

use core::hint::black_box;
use core::ops::ControlFlow;

use hadris_fat::embedded::{MountToken, Options};
use hadris_fs::{DirCursor, MountError, OpenOptions};
use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize};

/// A 1 GiB card whose reads and writes go nowhere the optimizer can see.
pub struct Card;

impl ErrorType for Card {
    type Error = core::convert::Infallible;
}

impl hadris_storage::sync::BlockDevice for Card {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        black_box(1 << 21)
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        black_box((first, buf));
        Ok(())
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        black_box((first, buf));
        Ok(())
    }
}

impl hadris_storage::local::BlockDevice for Card {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        black_box(1 << 21)
    }

    fn writable(&self) -> bool {
        true
    }

    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        black_box((first, buf));
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        black_box((first, buf));
        Ok(())
    }
}

pub mod sync {
    //! The sessions over a `hadris_storage::sync::BlockDevice`.

    use super::*;
    use hadris_fat::embedded::sync::Fat;
    use hadris_fat::exfat::embedded::sync::ExFat;
    use hadris_storage::sync::BlockDevice;

    /// Mounts a FAT volume. The result lands in the caller's frame, so
    /// this function's stack is what mounting takes.
    #[inline(never)]
    pub fn mount_fat<D: BlockDevice>(
        dev: D,
        token: &mut MountToken,
        options: Options,
    ) -> Result<Fat<'_, D>, MountError<D, D::Error>> {
        Fat::mount_with(dev, token, options)
    }

    /// A data logger: appends a line to a short-named file, lists the
    /// directory and reads the file back, then unmounts.
    #[inline(never)]
    pub fn log<D: BlockDevice>(dev: D, options: Options) -> Option<D> {
        let mut token = MountToken::new();
        let mut fat = mount_fat(dev, &mut token, options).ok()?;
        let root = fat.root();
        let logs = match fat.open_dir(root, "LOGS") {
            Ok(dir) => dir,
            Err(_) => fat.create_dir(root, "LOGS").ok()?,
        };
        let log = fat
            .open(
                logs,
                "BOOT.TXT",
                OpenOptions::new().write().create().append(),
            )
            .ok()?;
        fat.write(&log, b"booted\n").ok()?;
        fat.close(log).ok()?;
        let mut count = 0;
        fat.list(logs, DirCursor::START, |_| {
            count += 1;
            ControlFlow::Continue(())
        })
        .ok()?;
        let file = fat.open(logs, "BOOT.TXT", OpenOptions::new().read()).ok()?;
        let mut buf = [0u8; 64];
        let len = fat.read(&file, &mut buf).ok()?;
        black_box((count, &buf[..len]));
        fat.close(file).ok()?;
        fat.unmount().ok()
    }

    /// Every kind of call: long names, a nested tree, append, list, read,
    /// rename and recursive removal, then unmounts.
    #[inline(never)]
    pub fn fat<D: BlockDevice>(dev: D, options: Options) -> Option<D> {
        let mut token = MountToken::new();
        let mut fat = mount_fat(dev, &mut token, options).ok()?;
        let root = fat.root();
        let logs = fat.create_dir_all(root, "data/logs").ok()?;
        let log = fat
            .open(
                logs,
                "boot.txt",
                OpenOptions::new().write().create().append(),
            )
            .ok()?;
        fat.write(&log, b"booted\n").ok()?;
        fat.close(log).ok()?;
        let mut count = 0;
        fat.list(logs, DirCursor::START, |_| {
            count += 1;
            ControlFlow::Continue(())
        })
        .ok()?;
        let file = fat.open(logs, "Boot.TXT", OpenOptions::new().read()).ok()?;
        let mut buf = [0u8; 64];
        let len = fat.read(&file, &mut buf).ok()?;
        black_box((count, &buf[..len]));
        fat.close(file).ok()?;
        fat.rename(logs, "boot.txt", root, "last boot.txt").ok()?;
        fat.remove_dir_all(root, "data").ok()?;
        fat.unmount().ok()
    }

    /// Mounts an exFAT volume, as `mount_fat` does.
    #[inline(never)]
    pub fn mount_exfat<D: BlockDevice>(
        dev: D,
        token: &mut MountToken,
    ) -> Result<ExFat<'_, D>, MountError<D, D::Error>> {
        ExFat::mount(dev, token)
    }

    /// Reads the label and free space, lists the root and reads a file
    /// when it is there, then unmounts.
    #[inline(never)]
    pub fn exfat<D: BlockDevice>(dev: D) -> Option<D> {
        let mut token = MountToken::new();
        let mut exfat = mount_exfat(dev, &mut token).ok()?;
        let mut label = [0u8; 44];
        let label = exfat.label(&mut label).ok()?;
        let free = exfat.stats().ok()?.free_blocks();
        black_box((label, free));
        let root = exfat.root();
        let mut count = 0;
        exfat
            .list(root, DirCursor::START, |_| {
                count += 1;
                ControlFlow::Continue(())
            })
            .ok()?;
        black_box(count);
        if let Ok(dir) = exfat.open_dir(root, "DCIM") {
            if let Ok(file) = exfat.open(dir, "photo.jpg", OpenOptions::new().read()) {
                let mut buf = [0u8; 64];
                let len = exfat.read(&file, &mut buf).ok()?;
                black_box(&buf[..len]);
                exfat.close(file).ok()?;
            }
        }
        Some(exfat.unmount())
    }
}

pub mod local {
    //! The FAT session over a `hadris_storage::local::BlockDevice`.

    use super::*;
    use hadris_fat::embedded::r#async::Fat;
    use hadris_storage::local::BlockDevice;

    /// Mounts a FAT volume.
    #[inline(never)]
    pub async fn mount_fat<D: BlockDevice>(
        dev: D,
        token: &mut MountToken,
        options: Options,
    ) -> Result<Fat<'_, D>, MountError<D, D::Error>> {
        Fat::mount_with(dev, token, options).await
    }

    /// The sync `fat` session, awaited.
    #[inline(never)]
    pub async fn fat<D: BlockDevice>(dev: D, options: Options) -> Option<D> {
        let mut token = MountToken::new();
        let mut fat = mount_fat(dev, &mut token, options).await.ok()?;
        let root = fat.root();
        let logs = fat.create_dir_all(root, "data/logs").await.ok()?;
        let log = fat
            .open(
                logs,
                "boot.txt",
                OpenOptions::new().write().create().append(),
            )
            .await
            .ok()?;
        fat.write(&log, b"booted\n").await.ok()?;
        fat.close(log).await.ok()?;
        let mut count = 0;
        fat.list(logs, DirCursor::START, |_| {
            count += 1;
            ControlFlow::Continue(())
        })
        .await
        .ok()?;
        let file = fat
            .open(logs, "Boot.TXT", OpenOptions::new().read())
            .await
            .ok()?;
        let mut buf = [0u8; 64];
        let len = fat.read(&file, &mut buf).await.ok()?;
        black_box((count, &buf[..len]));
        fat.close(file).await.ok()?;
        fat.rename(logs, "boot.txt", root, "last boot.txt")
            .await
            .ok()?;
        fat.remove_dir_all(root, "data").await.ok()?;
        fat.unmount().await.ok()
    }

    /// Polls `future` to completion with a waker that does nothing, as an
    /// executor would for a device that is always ready.
    pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
        let mut context = core::task::Context::from_waker(core::task::Waker::noop());
        let mut future = core::pin::pin!(future);
        loop {
            if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
                return out;
            }
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

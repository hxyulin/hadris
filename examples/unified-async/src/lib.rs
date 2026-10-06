//! A stable-Rust experiment in one asynchronous driver with two future contracts.
//!
//! A Send value alone cannot strengthen a local trait's future contract:
//!
//! ```compile_fail
//! use hadris_example_unified_async::{FileSystem, require_send};
//! fn generic<F: FileSystem + Send>(fs: &mut F, buf: &mut [u8]) {
//!     require_send(FileSystem::read(fs, 0, buf));
//! }
//! ```
//!
//! An empty marker trait does not strengthen that contract either:
//!
//! ```compile_fail
//! use hadris_example_unified_async::{FileSystem, require_send};
//! trait Marker: FileSystem + Send {}
//! impl<T: FileSystem + Send> Marker for T {}
//! fn generic<F: Marker>(fs: &mut F, buf: &mut [u8]) {
//!     require_send(FileSystem::read(fs, 0, buf));
//! }
//! ```
//!
//! Redeclaring the stronger method in a subtrait works, but requires qualifying
//! the method when both declarations are in scope:
//!
//! ```compile_fail
//! use hadris_example_unified_async::SendFileSystem;
//! fn generic<F: SendFileSystem>(fs: &mut F, buf: &mut [u8]) {
//!     let _ = fs.read(0, buf);
//! }
//! ```

//! Blanket conversion conflicts with a universal local reference implementation:
//!
//! ```compile_fail
//! trait Local {}
//! trait Strong: Send {}
//! impl<T: Strong + ?Sized> Local for T {}
//! impl<T: Local + ?Sized> Local for &mut T {}
//! ```
//!
//! Even a concrete Send driver can contain a non-Send operation future:
//!
//! ```compile_fail
//! use hadris_example_unified_async::{Device, Fs, require_send};
//! struct HiddenRc;
//! impl Device for HiddenRc {
//!     async fn read(&mut self, _: usize, _: &mut [u8]) -> usize {
//!         let state = std::rc::Rc::new(());
//!         std::future::pending::<()>().await;
//!         drop(state);
//!         0
//!     }
//!     async fn flush(&mut self) {}
//! }
//! fn send_value<T: Send>(_: &T) {}
//! let mut fs = Fs::new(HiddenRc);
//! send_value(&fs);
//! require_send(fs.read(0, &mut []));
//! ```

//! Generic device adapters introduce the same blanket-implementation conflict:
//!
//! ```compile_fail
//! trait Local {}
//! trait Strong: Send {}
//! impl<T: Strong> Local for T {}
//! struct Partition<D>(D);
//! impl<D: Local> Local for Partition<D> {}
//! impl<D: Strong> Strong for Partition<D> {}
//! ```

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]

use core::future::Future;

/// Device operations whose futures may be non-Send.
pub trait Device {
    /// Reads bytes starting at `offset`.
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize>;
    /// Flushes pending writes.
    fn flush(&mut self) -> impl Future<Output = ()>;
}

/// The stronger device contract. A device author implements this or `Device`.
pub trait SendDevice: Send {
    /// Reads bytes with a Send future.
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> + Send;
    /// Flushes with a Send future.
    fn flush(&mut self) -> impl Future<Output = ()> + Send;
}

impl<D: SendDevice + ?Sized> Device for D {
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> {
        SendDevice::read(self, offset, buf)
    }

    fn flush(&mut self) -> impl Future<Output = ()> {
        SendDevice::flush(self)
    }
}

impl<D: SendDevice + ?Sized> SendDevice for &mut D {
    async fn read(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        SendDevice::read(*self, offset, buf).await
    }

    async fn flush(&mut self) {
        SendDevice::flush(*self).await;
    }
}

/// A device borrow for local implementations, avoiding overlap with the Send bridge.
pub struct Borrowed<'a, D: ?Sized>(pub &'a mut D);

impl<D: Device + ?Sized> Device for Borrowed<'_, D> {
    async fn read(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        Device::read(self.0, offset, buf).await
    }

    async fn flush(&mut self) {
        Device::flush(self.0).await;
    }
}

/// One concrete driver for local and Send devices.
pub struct Fs<D>(D);

impl<D: Device> Fs<D> {
    /// Constructs the driver without selecting a mode.
    pub fn new(device: D) -> Self {
        Self(device)
    }

    async fn read_chunk(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        Device::read(&mut self.0, offset, buf).await
    }

    /// Reads in bounded chunks, using the same algorithm in both contracts.
    pub async fn read(&mut self, mut offset: usize, mut buf: &mut [u8]) -> usize {
        let mut total = 0;
        while !buf.is_empty() {
            let chunk = buf.len().min(4);
            let n = self.read_chunk(offset, &mut buf[..chunk]).await;
            total += n;
            offset += n;
            buf = &mut buf[n..];
            if n < chunk {
                break;
            }
        }
        total
    }

    /// Flushes device state, with Send capability following the device contract.
    pub async fn sync(&mut self) {
        Device::flush(&mut self.0).await;
    }
}

/// The ordinary async filesystem contract.
pub trait FileSystem {
    /// Reads file bytes.
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize>;
    /// Flushes pending changes.
    fn sync(&mut self) -> impl Future<Output = ()>;
}

impl<D: Device> FileSystem for Fs<D> {
    async fn read(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        self.read(offset, buf).await
    }
    async fn sync(&mut self) {
        self.sync().await;
    }
}

/// A filesystem refinement exposing explicit Send future guarantees.
pub trait SendFileSystem: FileSystem + Send {
    /// Reads file bytes with a Send future.
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> + Send;
    /// Flushes pending changes with a Send future.
    fn sync(&mut self) -> impl Future<Output = ()> + Send;
}

impl<D: SendDevice> SendFileSystem for Fs<D> {
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> + Send {
        self.read(offset, buf)
    }

    fn sync(&mut self) -> impl Future<Output = ()> + Send {
        self.sync()
    }
}

/// Compile-time witness for a future's Send guarantee.
pub fn require_send(_: impl Future + Send) {}

/// A generic caller needs one stronger bound and qualifies the refined method.
pub async fn generic_send_read<F: SendFileSystem>(
    fs: &mut F,
    offset: usize,
    buf: &mut [u8],
) -> usize {
    SendFileSystem::read(fs, offset, buf).await
}

/// An alternative retaining independent local and Send trait contracts.
pub mod peer {
    use super::*;

    /// The Send filesystem contract without inheriting duplicate local methods.
    pub trait FileSystem: Send {
        /// Reads bytes with a Send future.
        fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> + Send;
    }

    impl<D: SendDevice> FileSystem for Fs<D> {
        fn read(&mut self, offset: usize, buf: &mut [u8]) -> impl Future<Output = usize> + Send {
            self.read(offset, buf)
        }
    }

    /// One bound permits normal method syntax, but does not imply the local trait.
    pub async fn generic_read<F: FileSystem>(fs: &mut F, offset: usize, buf: &mut [u8]) -> usize {
        fs.read(offset, buf).await
    }
}

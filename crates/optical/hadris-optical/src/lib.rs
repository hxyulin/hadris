//! # Hadris Optical
//!
//! Detection and opening of ISO 9660 and UDF images on any
//! `hadris_storage` block device, next to the format crates it builds on.
//!
//! [`detect`] reports ISO 9660 and UDF independently, because a bridge
//! image holds both. `OpenOpticalImage`, in each mode
//! (`sync::OpenOpticalImage`, `r#async::OpenOpticalImage`), detects and mounts the filesystem an
//! [`OpenPolicy`] selects and implements the `hadris_fs` `FsDriver` trait
//! read-only by delegating to it. A failed open gives the device back in
//! a `hadris_fs::MountError`. Neither needs an allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fs::sync::DriverExt;
//! use hadris_fs::tree::{Content, Tree};
//! use hadris_optical::sync::OpenOpticalImage;
//! use hadris_optical::{OpenPolicy, OpticalFormat};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut tree = Tree::new();
//! tree.add_file("readme.txt", Content::bytes("hello"))?;
//! let options = hadris_optical::udf::UdfOptions::default();
//! let size = hadris_optical::udf::sync::plan(&tree, &options)?.size_bytes();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! hadris_optical::udf::sync::write(&mut dev, &tree, &options)?;
//!
//! let mut image = OpenOpticalImage::open(dev, OpenPolicy::PreferUdf)?;
//! assert_eq!(image.format(), OpticalFormat::Udf);
//! assert_eq!(image.read_to_vec("/readme.txt")?, b"hello");
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! The format crates are re-exported as [`iso`], [`udf`] and, with the
//! `cd` feature, `cd`.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and `hadris_storage::host::FileDevice` |
//! | `alloc` | via `std` | `PathError` conversions and the ISO 9660 and UDF writers |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API with `Send` futures in `r#async` |
//! | `cd` | No | Re-exports `hadris-cd`, the hybrid image writer; implies `alloc` |
//!
//! No feature changes what an item does.

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod detect;
mod error;
mod image;

pub use error::{Detail, OpticalFormat};
pub use image::OpenPolicy;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }

    macro_rules! impl_optical_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    use crate::detect::sync::detect;
    use hadris_iso::sync::{IsoImage, IsoView};
    use hadris_storage::sync::BlockDevice;
    use hadris_udf::sync::UdfFs;

    #[path = "open.rs"]
    mod open;
    pub use open::OpenOpticalImage;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! impl_optical_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    use crate::detect::r#async::detect;
    use hadris_iso::r#async::{IsoImage, IsoView};
    use hadris_storage::r#async::BlockDevice;
    use hadris_udf::r#async::UdfFs;

    #[allow(clippy::duplicate_mod)]
    #[path = "open.rs"]
    mod open;
    pub use open::OpenOpticalImage;
}

/// ISO 9660 images, which `OpenOpticalImage` opens as an `IsoView`.
pub use hadris_iso as iso;

/// Universal Disk Format volumes, which `OpenOpticalImage` opens as
/// `UdfFs`.
pub use hadris_udf as udf;

/// Hybrid ISO 9660 and UDF images.
#[cfg(feature = "cd")]
#[cfg_attr(docsrs, doc(cfg(feature = "cd")))]
pub use hadris_cd as cd;

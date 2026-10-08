#[allow(unused_macros)]
macro_rules! send_filesystem { ($($item:tt)*) => { $($item)* }; }
#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_apfs::async_::ApfsFs;
use hadris_apfs::async_::Container as ApfsContainer;
#[cfg(feature = "alloc")]
use hadris_fat::async_::FatFs;
#[cfg(feature = "alloc")]
use hadris_fat::exfat::async_::ExFatFs;
use hadris_fat_raw::exfat::io::async_ as exio;
use hadris_fat_raw::io::async_ as rawio;
#[cfg(feature = "alloc")]
use hadris_fs::local::FileSystem;
use hadris_iso::async_::IsoFs;
use hadris_storage::async_::BlockDevice;
use hadris_udf::async_::UdfFs;

#[cfg(all(feature = "alloc", feature = "write"))]
use hadris_fat::{async_::format as fat_format, exfat::async_::format as exfat_format};
#[cfg(all(feature = "alloc", feature = "write"))]
use hadris_udf::async_::write as udf_write;

#[path = "open.rs"]
mod open;
pub use open::detect;
#[cfg(all(feature = "alloc", feature = "write"))]
pub use open::format;
#[cfg(feature = "alloc")]
pub use open::{AnyFs, open};

#[cfg(all(feature = "alloc", feature = "unstable-apfs"))]
pub use open::open_apfs;

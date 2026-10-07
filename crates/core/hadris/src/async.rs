#[allow(unused_macros)]
macro_rules! send_filesystem { ($($item:tt)*) => { $($item)* }; }
#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_apfs::r#async::ApfsFs;
use hadris_apfs::r#async::Container as ApfsContainer;
#[cfg(feature = "alloc")]
use hadris_fat::r#async::FatFs;
#[cfg(feature = "alloc")]
use hadris_fat::exfat::r#async::ExFatFs;
use hadris_fat_raw::exfat::io::r#async as exio;
use hadris_fat_raw::io::r#async as rawio;
#[cfg(feature = "alloc")]
use hadris_fs::local::FileSystem;
use hadris_iso::r#async::IsoFs;
use hadris_storage::async_::BlockDevice;
use hadris_udf::r#async::UdfFs;

#[path = "open.rs"]
mod open;
pub use open::detect;
#[cfg(feature = "alloc")]
pub use open::{AnyFs, open};

#[cfg(all(feature = "alloc", feature = "unstable-apfs"))]
pub use open::open_apfs;

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

#[cfg(feature = "alloc")]
use hadris_fat::r#async::FatFs;
#[cfg(feature = "alloc")]
use hadris_fat::exfat::r#async::ExFatFs;
use hadris_fat_raw::exfat::io::r#async as exio;
use hadris_fat_raw::io::r#async as rawio;
#[cfg(feature = "alloc")]
use hadris_fs::r#async::FileSystem;
use hadris_iso::r#async::IsoFs;
use hadris_storage::r#async::BlockDevice;
use hadris_udf::r#async::UdfFs;

#[path = "open.rs"]
mod open;
pub use open::detect;
#[cfg(feature = "alloc")]
pub use open::{AnyFs, open};

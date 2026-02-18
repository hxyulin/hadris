#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

pub mod error;
pub mod file;
pub mod raw;

// ExFAT (WIP, stays at crate root for now)
#[cfg(feature = "exfat")]
pub mod exfat;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous FAT filesystem API.
    //!
    //! All I/O operations use synchronous `Read`/`Write`/`Seek` traits.

    pub use hadris_io::sync::{Read, Write, Seek, ReadExt, Parsable, Writable};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};
    pub use hadris_io::Result as IoResult;

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    macro_rules! sync_only {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! async_only {
        ($($item:tt)*) => { };
    }

    #[path = "."]
    mod __inner {
        pub mod io;
        pub mod fat_table;
        pub mod fs;
        pub mod dir;
        pub mod read;
        pub mod write;
        #[cfg(feature = "cache")]
        pub mod cache;
        #[cfg(feature = "write")]
        pub mod format;
        #[cfg(feature = "tool")]
        pub mod tool;
    }
    pub use __inner::*;

    // Convenience re-exports for backwards compatibility
    pub use __inner::fs::FatFs;
    pub use __inner::fat_table::{Fat, FatType, Fat12, Fat16, Fat32};
    pub use __inner::dir::{DirectoryEntry, FatDir, FileEntry};
    pub use __inner::read::FatFsReadExt;
    #[cfg(feature = "write")]
    pub use __inner::write::{FatFsWriteExt, FatDateTime};
    #[cfg(feature = "tool")]
    pub use __inner::tool::analysis::FatAnalysisExt;
    #[cfg(feature = "tool")]
    pub use __inner::tool::verify::FatVerifyExt;
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous FAT filesystem API.
    //!
    //! All I/O operations use async `Read`/`Write`/`Seek` traits.

    pub use hadris_io::r#async::{Read, Write, Seek, ReadExt, Parsable, Writable};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};
    pub use hadris_io::Result as IoResult;

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! sync_only {
        ($($item:tt)*) => { };
    }

    macro_rules! async_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[path = "."]
    mod __inner {
        pub mod io;
        pub mod fat_table;
        pub mod fs;
        pub mod dir;
        pub mod read;
        pub mod write;
        #[cfg(feature = "cache")]
        pub mod cache;
        #[cfg(feature = "write")]
        pub mod format;
        #[cfg(feature = "tool")]
        pub mod tool;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-exports from shared types
pub use error::{FatError, Result};
pub use raw::*;

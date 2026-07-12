//! Parses HADRIS_FAT_CACHE_SIZE at build time and sets rustc-env FAT_CACHE_WINDOW_SIZE_BYTES.
//! The crate reads it with env!("FAT_CACHE_WINDOW_SIZE_BYTES").parse(). Example: 8MiB → 8388608.
//! Panics if the value is set and not a valid non-negative integer.

use std::env;

fn main() {
    let default_bytes = 4 * 1024 * 1024u64;
    let bytes = match env::var("HADRIS_FAT_CACHE_WINDOW_SIZE") {
        Ok(s) => parse_size::parse_size(s.trim()).unwrap_or_else(|_| {
            panic!("HADRIS_FAT_CACHE_WINDOW_SIZE must be a valid storage unit (e.g. 4MiB, 4194304)")
        }),
        Err(_) => default_bytes,
    };

    println!("cargo:rustc-env=FAT_CACHE_WINDOW_SIZE_BYTES={}", bytes);
}

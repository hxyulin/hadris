/// Algorithms
pub mod alg;
/// Partitions
pub mod part;
/// Strings
pub mod str;
/// Types
pub mod types;

/// A generic boot sector that informs the user they are loading the image incorrectly.
///
/// This is generated using the code in the `boot_sector` directory. See the README for more information.
/// Currently this is required to be maually compiled, and tested. 
/// TODO: Make this a build script that generates the binary, and verify with a static analysis tool.
pub static BOOT_SECTOR_BIN: &[u8] = include_bytes!("boot_sector.bin");

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert!(BOOT_SECTOR_BIN.len() == 512);
}

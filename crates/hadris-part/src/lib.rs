#![no_std]

//! A crate for working with partitions.
//! Currently this supports MBR and GPT partitioned disks.

/// A platform-indepedent, partition
pub struct Partition {
    start: u64,
    size: u64,
}

pub trait Disk {
    fn get_partitions(&self) -> impl Iterator<Item = Partition>;
}

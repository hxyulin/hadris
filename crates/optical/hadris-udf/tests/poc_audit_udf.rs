//! Regression tests for audit-found write-path robustness bugs.
#![cfg(all(feature = "std", feature = "sync", feature = "write"))]

use std::io::Cursor;

use hadris_udf::Error;
use hadris_udf::sync::FileType;
use hadris_udf::sync::descriptor::ShortAllocationDescriptor;
use hadris_udf::sync::write::{SimpleDir, UdfWriteOptions, UdfWriter};

/// 234 short allocation descriptors exactly fill the 2048-byte File Entry
/// buffer (allocation descriptors start at offset 176); more must return an
/// error instead of panicking on the slice index.
#[test]
fn write_file_entry_rejects_oversized_allocation_descriptors() {
    let fits = vec![ShortAllocationDescriptor::default(); 234];
    let mut udf = UdfWriter::new(Cursor::new(Vec::new()), UdfWriteOptions::default());
    udf.write_file_entry(0, FileType::RegularFile, 0, &fits, 16)
        .expect("234 descriptors fit in one sector");

    let too_many = vec![ShortAllocationDescriptor::default(); 235];
    let mut udf = UdfWriter::new(Cursor::new(Vec::new()), UdfWriteOptions::default());
    let result = udf.write_file_entry(0, FileType::RegularFile, 0, &too_many, 16);
    assert!(
        matches!(result, Err(Error::TooManyAllocationDescriptors)),
        "expected TooManyAllocationDescriptors, got {result:?}"
    );
}

fn nested_dirs(depth: usize) -> SimpleDir {
    let mut tree = SimpleDir::new("d");
    for _ in 0..depth {
        let mut parent = SimpleDir::new("d");
        parent.add_dir(tree);
        tree = parent;
    }
    tree
}

/// The formatter recurses once per directory nesting level; deeply nested
/// trees must be rejected with an error instead of overflowing the stack.
#[test]
fn create_rejects_excessively_nested_directories() {
    let tree = nested_dirs(200);
    let result = UdfWriter::create(Cursor::new(Vec::new()), &tree, UdfWriteOptions::default());
    assert!(
        matches!(result, Err(Error::DirectoryNestingTooDeep)),
        "expected DirectoryNestingTooDeep, got {}",
        result.err().map(|e| e.to_string()).unwrap_or_default()
    );
}

/// Realistic nesting depth still formats successfully.
#[test]
fn create_accepts_reasonably_nested_directories() {
    let tree = nested_dirs(32);
    UdfWriter::create(Cursor::new(Vec::new()), &tree, UdfWriteOptions::default())
        .expect("32 levels of nesting should format");
}

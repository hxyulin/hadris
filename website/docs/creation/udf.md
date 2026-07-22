---
title: Create UDF filesystems
---

# Create UDF filesystems

`hadris-udf` creates mastered, read-only Type-1 UDF images on a seekable target.
The high-level tree API is appropriate for standalone images; `hadris-cd`
should be used when authoring a shared ISO/UDF bridge image.

## Dependency

```toml
[dependencies]
hadris-udf = { version = "2.0.0-rc.3", features = ["write", "sync"] }
```

## Create a directory tree

`SimpleDir` and `SimpleFile` own their payloads. Sort the tree when deterministic
directory ordering matters.

```rust
use std::fs::OpenOptions;

use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut root = SimpleDir::root();
    root.add_file(SimpleFile::new(
        "README.txt",
        b"Hello from a UDF image\n".to_vec(),
    ));

    let mut docs = SimpleDir::new("docs");
    docs.add_file(SimpleFile::new("guide.txt", b"UDF guide\n".to_vec()));
    root.add_dir(docs);
    root.sort();

    let target = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("volume.udf")?;

    let output = UdfWriter::create(target, &root, UdfWriteOptions::default())?;
    println!("wrote {} sectors", output.sectors_written);
    let _target = output.into_inner();
    Ok(())
}
```

The output reports the number of 2048-byte sectors used and returns the target.
Unlike FAT formatting, the standalone UDF writer lays out and grows an ordinary
file as it writes, so it does not need to be pre-sized.

## Select a mastered revision

```rust
use hadris_udf::UdfRevision;
use hadris_udf::write::UdfWriteOptions;

let options = UdfWriteOptions {
    volume_id: "ARCHIVE_2026".into(),
    revision: UdfRevision::V2_01,
    ..UdfWriteOptions::default()
};
```

The revision describes a mastered/read-only Type-1 image. It does not enable
packet writing, VAT, sparing, metadata partitions, or pseudo-overwrite. Choose
the oldest revision that provides the compatibility your consumers need, and
validate it with the tools used by those consumers.

## Unicode names and limits

Filenames are encoded with OSTA Compressed Unicode (CS0). Hadris selects 8-bit
compression for names representable in one byte per character and 16-bit
compression otherwise. A filename whose encoded FID identifier exceeds 255
bytes is rejected rather than truncated.

The high-level API currently owns every file payload in memory. Very large
payloads and live streaming are outside this convenience surface.

## Create an in-memory image

Preallocate enough space when the target is a bounded cursor:

```rust
use std::io::Cursor;
use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

let mut root = SimpleDir::root();
root.add_file(SimpleFile::new("hello.txt", b"hello\n".to_vec()));

let mut storage = vec![0_u8; 8 * 1024 * 1024];
let cursor = Cursor::new(storage.as_mut_slice());
let output = UdfWriter::create(cursor, &root, UdfWriteOptions::default())?;
assert!(output.sectors_written > 0);
# Ok::<(), hadris_udf::UdfError>(())
```

## Author an ISO/UDF bridge

Do not independently concatenate ISO and UDF images. A bridge must coordinate
descriptor locations, directory ICBs, and payload extents. Use the bridge crate
or CLI:

```bash
hadris-cd create image-root bridge.iso
hadris-cd verify bridge.iso
```

The verifier compares both namespace trees and confirms that shared files are
readable through ISO and UDF.

## Validate the result

```bash
udfinfo volume.udf
7z l volume.udf
hadris-udf info volume.udf
```

For interoperability work, also create reference images with `mkudffs` and
confirm that Hadris can read them. Validation should cover every UDF revision
your application accepts.

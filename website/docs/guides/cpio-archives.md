---
title: Read and create CPIO archives
---

# Read and create CPIO archives

Hadris supports the newc (`070701`) and newc CRC (`070702`) formats commonly
used by Linux initramfs images.

```toml
[dependencies]
hadris-cpio = "2.1.0"
```

## Stream archive entries

```rust,no_run
use hadris_cpio::CpioArchiveReader;
use std::{fs::File, io::BufReader};

fn main() -> hadris_cpio::Result<()> {
    let input = BufReader::new(File::open("archive.cpio")?);
    let mut archive = CpioArchiveReader::new(input);

    while let Some(entry) = archive.next_entry_alloc()? {
        let name = entry.name_str().unwrap_or("<non-UTF-8>");
        println!("{} ({} bytes)", name, entry.file_size());
        archive.skip_entry_data_owned(&entry)?;
    }

    Ok(())
}
```

Every returned entry is followed immediately by its data. Before requesting
the next entry, either read the current payload or skip it. Allocation-free
consumers can use `next_entry` with a caller-provided filename buffer.

## Read a file payload

```rust,no_run
# use hadris_cpio::CpioArchiveReader;
# use std::{fs::File, io::BufReader};
# fn run() -> hadris_cpio::Result<()> {
let mut archive = CpioArchiveReader::new(BufReader::new(File::open("archive.cpio")?));
while let Some(entry) = archive.next_entry_alloc()? {
    if entry.name() == b"etc/hostname" {
        let bytes = archive.read_entry_data_alloc(&entry)?;
        println!("{}", String::from_utf8_lossy(&bytes));
        break;
    }
    archive.skip_entry_data_owned(&entry)?;
}
# Ok(())
# }
```

## Create an archive

```rust,no_run
use hadris_cpio::{CpioArchiveWriter, CpioWriteOptions, FileTree};
use std::{fs::File, io::BufWriter, path::Path};

fn main() -> hadris_cpio::Result<()> {
    let tree = FileTree::from_fs(Path::new("./root"))?;
    let output = BufWriter::new(File::create("archive.cpio")?);
    CpioArchiveWriter::new(output, CpioWriteOptions::default()).finish(&tree)?;
    Ok(())
}
```

For initramfs-specific guidance, see [Build a CPIO initramfs](./build-initramfs.md).

---
title: Read and create CPIO archives
---

# Read and create CPIO archives

Hadris reads the newc (`070701`), newc CRC (`070702`), odc (`070707`) and
old binary formats, and writes newc, newc CRC and odc. Linux initramfs
images use newc.

```toml
[dependencies]
hadris-cpio = "3.0.0-rc.1"
hadris-fs = "3.0.0-rc.1"
hadris-io = "3.0.0-rc.1"
```

## Stream archive entries

```rust,no_run
use hadris_cpio::sync::CpioReader;
use hadris_io::StdIo;
use std::{fs::File, io::BufReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = StdIo::new(BufReader::new(File::open("archive.cpio")?));
    let mut archive = CpioReader::new(input);

    while let Some(entry) = archive.next_entry()? {
        let name = entry.name_str().unwrap_or("<non-UTF-8>");
        println!("{} ({} bytes)", name, entry.len());
    }

    Ok(())
}
```

`next_entry` returns an entry that borrows the reader. Data the caller does
not read is skipped by the next call, and CRC archives are checked either
way. The reader needs no allocator, so the same code runs in a bootloader.
An archive may end at an entry boundary without a trailer, as initramfs
allows; `ReaderOptions::with_strict_trailer` requires one, and
`continue_after_trailer` reads archives concatenated after a trailer.

## Read a file payload

```rust,no_run
# use hadris_cpio::sync::CpioReader;
# use hadris_io::StdIo;
# use hadris_io::sync::Read;
# use std::{fs::File, io::BufReader};
# fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut archive = CpioReader::new(StdIo::new(BufReader::new(File::open("archive.cpio")?)));
while let Some(mut entry) = archive.next_entry()? {
    if entry.name() == b"etc/hostname" {
        let mut bytes = vec![0; entry.len() as usize];
        entry.read_exact(&mut bytes)?;
        println!("{}", String::from_utf8_lossy(&bytes));
        break;
    }
}
# Ok(())
# }
```

## Create an archive

```rust,no_run
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::host::{self, TreeOptions};
use hadris_io::StdIo;
use std::{fs::File, io::BufWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tree, _) = host::read_tree("./root", &TreeOptions::new())?;
    let mut output = StdIo::new(BufWriter::new(File::create("archive.cpio")?));
    let options = CpioOptions::default().with_format(Format::Crc);
    let report = hadris_cpio::sync::write(&mut output, &tree, &options)?;
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    Ok(())
}
```

The report's warnings list metadata cpio cannot store, such as access
times, once per field with a count; `hadris_cpio::plan` returns the same
report without writing. Entries without a modification time get the
options' time (`CpioOptions::with_time`, 1980-01-01 by default). To write
entries one at a time, use `hadris_cpio::sync::Writer`: `append` takes a
`Node`, `append_hard_links` a group of names, and `append_file` streams
data of a known length through an `EntryWriter`; `finish` writes the
trailer. `hadris_cpio::sync::read_tree` reads an archive back into a
`Tree`.

For initramfs-specific guidance, see [Build a CPIO initramfs](./build-initramfs.md).

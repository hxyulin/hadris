# Hadris CPIO

cpio archives for Linux initramfs, RPM payloads and `cpio(1)`, in pure Rust:
an allocation-free streaming reader and a streaming writer that also takes
the shared `hadris_fs::tree::Tree` input of the other Hadris writers.

## Features

- Reads `newc` (`070701`), `newc` with checksums (`070702`), `odc`
  (`070707`) and old binary archives in either byte order.
- Writes `newc`, `newc` with checksums and `odc`.
- Regular files, directories, symlinks, hard links (GNU cpio layout), device
  nodes, FIFOs and sockets.
- Works on pipes: the reader and writer need only `Read` and `Write`.
- The same API blocking (`sync`) and asynchronous with `Send` futures
  (`r#async`).
- `no_std`; the reader needs no allocator.

## Reading an archive

```rust,no_run
use std::fs::File;
use std::io::BufReader;
use hadris_cpio::sync::CpioReader;
use hadris_io::StdIo;
use hadris_io::sync::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("initramfs.cpio")?;
    let mut reader = CpioReader::new(StdIo::new(BufReader::new(file)));
    while let Some(mut entry) = reader.next_entry()? {
        println!("{} ({} bytes)", entry.name_str().unwrap_or("?"), entry.len());
        if entry.name() == b"etc/hostname" {
            let mut data = vec![0; entry.len() as usize];
            entry.read_exact(&mut data)?;
        }
    }
    Ok(())
}
```

Data left unread is skipped by the next `next_entry`, and `070702`
checksums are verified whether the data is read or skipped. An archive may
end at an entry boundary without a trailer, as the initramfs format allows;
`ReaderOptions::with_strict_trailer` requires one. `continue_after_trailer`
reads archives concatenated after a trailer, such as a microcode archive
before the main initramfs.

## Writing an archive

```rust,no_run
use std::fs::File;
use std::io::BufWriter;
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::tree::{FromFsOptions, Tree};
use hadris_io::StdIo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tree = Tree::from_fs("./rootfs", FromFsOptions::new())?;
    let mut out = StdIo::new(BufWriter::new(File::create("initramfs.cpio")?));
    let options = CpioOptions::default().with_format(Format::Newc);
    let report = hadris_cpio::sync::write(&mut out, &tree, &options)?;
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    Ok(())
}
```

`CpioWriter` appends entries one at a time (`append`, `append_hard_links`,
`write_tree`) and `finish` writes the trailer. Metadata cpio cannot store
(times other than the modification time, sub-second parts, attributes) is
listed in the report's warnings. Values that do not fit a header field fail
before the entry is written. `CpioOptions::with_format(Format::Odc)` writes
`odc`, whose device numbers are stored as `major << 8 | minor`.

The header layouts are in `hadris_cpio::raw`.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
| `alloc` | via `std` | The writer and the `Tree` input |
| `sync` | yes | The blocking API in `sync` |
| `async` | no | The asynchronous API with `Send` futures in `r#async` |

A bootloader that only reads uses
`default-features = false, features = ["sync"]`.

## References

- `cpio(5)`
- Linux `Documentation/driver-api/early-userspace/buffer-format.rst`
- GNU cpio manual

## Documentation

- [Read and create CPIO archives](https://hxyulin.github.io/hadris/guides/cpio-archives)
- [Build a CPIO initramfs](https://hxyulin.github.io/hadris/guides/build-initramfs)
- [API reference](https://docs.rs/hadris-cpio)

## License

This project is licensed under the [MIT license](../../../LICENSE-MIT).

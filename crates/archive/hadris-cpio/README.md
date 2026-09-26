# Hadris CPIO

cpio archives for Linux initramfs, RPM payloads and `cpio(1)`, in pure Rust:
an allocation-free streaming reader and a streaming writer that also takes
the shared `hadris_fs::Tree` input of the other Hadris writers.

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
        println!("{} ({} bytes)", entry.path_str().unwrap_or("?"), entry.len());
        if entry.path() == b"etc/hostname" {
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
`ReaderOptions::with_strict_trailer` requires one. `next_segment()`
reads archives concatenated after a trailer, such as a microcode archive
before the main initramfs.

`CpioReader::with_buffer(input, &mut storage[..], options)` uses caller-owned
name storage; capacity includes the terminating NUL, including for the
`TRAILER!!!` name (11 bytes). Oversized encoded names return `LimitExceeded`.
`Entry::offset()` and `data_offset()` are measured from the supplied stream's
start. After `next_entry()` returns `None`, call `next_segment()` to skip zero
padding and test whether another segment follows. A true result only means
bytes follow; `next_entry()` validates their header. To recover the stream
without losing the peeked byte, use `into_parts()` and replay its optional
byte before reading the returned stream. `into_inner()` discards that byte.

## Writing an archive

```rust,no_run
use std::fs::File;
use std::io::BufWriter;
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::host::{self, TreeOptions};
use hadris_io::StdIo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tree, _skipped) = host::read_tree("./rootfs", &TreeOptions::new())?;
    let mut out = StdIo::new(BufWriter::new(File::create("initramfs.cpio")?));
    let options = CpioOptions::default().with_format(Format::Newc);
    let report = hadris_cpio::sync::write(&mut out, &tree, &options)?;
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    Ok(())
}
```

`hadris_cpio::plan` returns the same report without writing: the archive
size, where each file's data starts, and the metadata cpio cannot store
(times other than the modification time, sub-second parts, attributes).
`Writer` appends entries one at a time (`append`, `append_hard_links`, and
`append_file` for data produced while writing) and `finish` writes the
trailer and returns the stream with the report. `CpioOptions::with_time`
sets the modification time of entries that set none, such as
`SOURCE_DATE_EPOCH`. `sync::read_tree` reads an archive back into a
`Tree`. Values that do not fit a header field fail
before the entry is written. `CpioOptions::with_format(Format::Odc)` writes
`odc`, whose device numbers are stored as `major << 8 | minor`.

The header layouts are in `hadris_cpio::raw`.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
| `alloc` | via `std` | The writer, `plan` and `read_tree` |
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

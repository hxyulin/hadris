# Hadris CLI

> **Experimental / not ready for production use.** This crate is a stub and is
> **not published** to crates.io (`publish = false`). Prefer the specialized CLIs:
>
> - [hadris-fat-cli](../hadris-fat-cli) (`fatutil`) — FAT filesystem operations
> - [hadris-iso-cli](../hadris-iso-cli) — ISO 9660 operations
> - [hadris-cpio-cli](../hadris-cpio-cli) (`cpioutil`) — CPIO archives
> - [hadris-udf-cli](../hadris-udf-cli) — UDF operations

## Current behavior

From a workspace checkout only:

```bash
cargo run -p hadris-cli -- path/to/fat.img
```

Opens a FAT image and debug-prints root directory entries. There is no
subcommand structure, partition support, or multi-format dispatch yet.

## Planned Features

- Unified interface for multiple filesystem types
- Partition table operations (MBR/GPT)
- Disk image manipulation

## License

Licensed under the [MIT license](../../LICENSE-MIT).

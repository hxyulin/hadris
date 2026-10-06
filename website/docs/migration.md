---
title: Migrate from 2.x
---

# Migrate from 2.x to 3.0.0-rc.1

This page gives the implementation order for a 2.4/2.5 application. The
[full migration guide and symbol tables](https://github.com/hxyulin/hadris/blob/main/docs/hadris-3.0.0-migration.md)
cover individual replacements, CLI flags and APFS additions in 2.5.

## Select one dependency

Follow the [rc.1 source installation recipe](./getting-started.md#get-the-release-candidate).
The umbrella reaches the same native APIs as individual format crates, including
`hadris::fat`, `hadris::iso`, `hadris::fs`, `hadris::storage` and `hadris::io`.
A known FAT image needs only `std`, `sync` and `fat`; add `write` for formatting.

The candidate is not published on crates.io yet. Use the Git source in the
recipes and an explicit `3.0.0-rc.1` requirement. Disable default features when
selecting formats or targeting `no_std`; do so for every direct Hadris dependency
if individual crates are used, because Cargo unifies their features.

## Replace storage, then filesystem calls

| V2 application code | V3 replacement |
|---|---|
| `FatVolume::open(file)` | `FatFs::mount(FileDevice::open(path)?, MountOptions::new())` |
| A custom `Read + Seek` filesystem source | `StreamDevice<ReadOnly<T>>` for a read-only source, or implement `BlockDevice` |
| `PartitionView` | `storage::Partition`, usually returned by `part::sync::open` |
| `root_dir()` and directory entries | `Volume::read_dir(path)`, or `FileSystem::readdir(node, cursor)` |
| Format-specific file handles | `Volume::open(path, OpenOptions)` |
| Per-crate filesystem errors | `Error<E>`, with `ErrorKind` and the device's own error |
| Format-specific image input trees | `fs::Tree`, `Node` and `Content` |
| `hadris-fat`, `hadris-iso` and other binaries | `hadris fat`, `hadris iso`, `hadris udf` and `hadris cpio` |

`FileDevice` defaults image files to 512-byte blocks and queries physical
devices' logical block size. Use `open_with_block_size` for a copied 4Kn GPT
image. Firmware adapters must normalize alignment and transfer limits;
[the custom-device guide](./guides/custom-io.md) includes a compiled fixed-buffer
example. CPIO remains a byte stream so it can read and write pipes.

[The FAT reader guide](./guides/read-fat-image.md) shows the complete host path.
[The FAT editor guide](./guides/modify-fat.md) shows writes and error reporting.

## Choose a driver or a volume

`FileSystem` takes `&mut self` and works on node IDs. `resolve`, `lookup` and
`parent` pin returned nodes; balance these with `forget`. An entry from
`readdir` does not itself pin a node. The same generic functions work on FAT,
exFAT, ISO and UDF.

`Volume` adds path resolution, a lock and file/directory handles. Use the sync
volume on a host with `std`, or the async volume with `alloc`. Allocation-only
sync applications use the bare driver. A written file should be closed
explicitly, followed by volume sync or driver unmount when durability matters.
Dropping a handle cannot report a flush or metadata error to the caller.

## Preserve the platform boundary

| V2 use case | V3 tier |
|---|---|
| FAT without `std`, with an allocator | `fat::sync::FatFs` or its `r#async` twin, with `alloc` |
| FAT without an allocator | `fat::embedded`, with `MountToken` and fixed file slots |
| exFAT without an allocator | Read-only `fat::exfat::embedded` |
| ISO/UDF without an allocator | Shared read-only drivers; image writers and convenience APIs may need `alloc` |
| Non-`Send` asynchronous firmware | Local device traits and embedded FAT/exFAT |

Device errors still require `core::error::Error + Send + Sync + 'static` in
every mode. `local` relaxes device and future bounds, not error bounds. The
missing general local async filesystem tier is tracked in
[issue #267](https://github.com/hxyulin/hadris/issues/267).

Mutations are not transactions. A device error or cancelled future can leave
partial changes. Confirm the [documented durability limits](https://github.com/hxyulin/hadris/blob/main/KNOWN_ISSUES.md)
when moving writable applications.

## Run a complete migration

```sh
cargo run --locked -p hadris-example-migrate-v3
```

The [example source](https://github.com/hxyulin/hadris/tree/main/examples/migrate-v3)
uses one Hadris dependency, formats a memory-backed FAT image, writes through
`Volume`, closes and unmounts, reads through the generic `FileSystem` API and
the embedded FAT API, then reads an ISO with the same generic function.
It builds its own inputs and verifies the contents. The
[other runnable examples](https://github.com/hxyulin/hadris/blob/main/examples/README.md)
cover partitioned boot media, extraction, streaming initramfs and VFS integration.

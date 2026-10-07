# Hadris examples

Runnable, task-oriented applications showing how the V3 crates fit together. Each directory is a small workspace package and is compiled by
`cargo check --workspace`.

## Start with the V3 migration

```sh
cargo run --locked -p hadris-example-migrate-v3
```

[`migrate-v3`](migrate-v3) uses only `hadris`, builds a FAT image, writes through
`Volume`, closes and unmounts, reads through the bare `FileSystem` driver and
embedded FAT handles, then reads an ISO through the same generic function.
It needs no input and runs in CI. The
[migration guide](../docs/hadris-3.0.0-migration.md) maps the V2 calls it replaces.

The tool examples `fat-list`, `partition-list` and `cpio-create` also use one
Hadris dependency with explicit formats. Workspace paths select this checkout;
[the installation guide](../website/docs/getting-started.md) gives the
`3.0.0-rc.1` crates.io recipe for an external application.

## Use cases

One example per kind of user in the
[action catalog](../docs/v3/actions.md). Each builds its own images, needs
no input, checks every result and exits non-zero on a mismatch, so CI runs
them as end-to-end tests. Each one's doc comment lists the catalog actions it
covers.

| Example | User | What it does |
|---|---|---|
| [`migrate-v3`](migrate-v3) | migration | One dependency, path handles, generic FAT/ISO reads and embedded FAT handles |
| [`apfs-inspect`](apfs-inspect) | inspection and VFS | Mounts a read-only APFS volume, checks inode/cursor operations and sparse/linked files, extracts through the generic tree API, and selects a named volume from a container |
| [`boot-media`](boot-media) | image builder | A GPT USB disk with a FAT ESP, a BIOS and UEFI hybrid ISO sharing one ESP, and an ISO/UDF bridge, each read back |
| [`initramfs`](initramfs) | image builder | A microcode segment and a root archive streamed to a pipe-like sink, read back segment by segment from a non-seekable reader |
| [`image-edit`](image-edit) | host tool | Formats FAT16 and exFAT image files, edits them through one generic function, unmounts, runs `check` and reopens read-only |
| [`extract`](extract) | host tool | Builds FAT, exFAT, ISO and UDF images from a host directory, then detects, opens and extracts each and compares it with the source |
| [`vfs-driver`](vfs-driver) | kernel or FUSE | `NodeId` operations, cursor-paged `readdir`, `forget` and errno mapping over a FAT32 partition, read-write then read-only |
| [`inspect`](inspect) | forensics | A write-blocked mount, file carving from extents, a read-only mount that changes nothing, backup boot sector recovery, checker findings and ISO identifiers |
| [`sd-logger`](sd-logger) | embedded | The no-allocator API logging to a FAT16 card, a card that write-protects itself mid-session, and an exFAT card read by firmware |

```bash
cargo run -p hadris-example-boot-media            # check in memory
cargo run -p hadris-example-boot-media -- out/    # also keep the images
```

`boot-media`, `initramfs`, `image-edit` and `extract` take an optional output
path to keep what they built.

## Tools

| Example | Purpose |
|---|---|
| [`fat-list`](fat-list) | List the root directory of a FAT12/16/32 image |
| [`volume-list`](volume-list) | Detect a FAT12/16/32, exFAT, ISO 9660 or UDF image with `hadris::sync::detect`, open it with `hadris::host::open` and print its tree through any `hadris-fs` driver |
| [`partition-list`](partition-list) | Detect and list MBR or GPT partitions |
| [`cpio-create`](cpio-create) | Build a newc/SVR4 CPIO archive from a directory |
| [`firmware`](firmware) | Firmware-shaped sessions on the embedded `Fat` and `ExFat`: `no_std` images for bare-metal targets, measured by `scripts/firmware-size.py`, and runnable on the host |

Run an example from the repository root:

```bash
cargo run -p hadris-example-fat-list -- disk.img
cargo run -p hadris-example-volume-list -- disk.img
cargo run -p hadris-example-partition-list -- disk.img
cargo run -p hadris-example-cpio-create -- rootfs/ initramfs.cpio
cargo run -p hadris-example-firmware --bin fat-log
```

Crate examples cover single-crate tasks:

| Example | Run with |
|---|---|
| Share a FAT volume between threads with `Volume` | `cargo run -p hadris-fat --example shared_volume -- disk.img` |
| Print the trees, volume name and boot catalog of an ISO | `cargo run -p hadris-iso --example read_iso -- image.iso` |
| Extract an ISO with `read_tree` and `host::write_tree` | `cargo run -p hadris-iso --example extract_files -- image.iso out/` |
| Write a BIOS and UEFI bootable hybrid ISO | `cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso` |

These programs favor readable error messages and conventional host filesystem
I/O. For `no_std`, async, and format-authoring variants, see the task-oriented
documentation site under [`website/docs/guides`](../website/docs/guides).

## Async contract prototype

[`async-device-contract`](async-device-contract) is an unpublished experiment
for common device borrowing, adapters and derived Send guarantees. Its probe
scripts compare GAT designs and compile the real ISO reader against the
experimental contract; production APIs remain unchanged.

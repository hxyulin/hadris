# Hadris examples

Runnable, task-oriented applications showing how the published crates fit
together. Each directory is a small workspace package and is compiled by
`cargo check --workspace`.

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

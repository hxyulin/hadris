---
title: FAT and exFAT on a microcontroller
---

# Use FAT and exFAT on a microcontroller

Firmware without an allocator uses the embedded API of `hadris-fat`:
`hadris_fat::embedded::sync::Fat` reads and writes FAT12, FAT16 and FAT32,
and `hadris_fat::exfat::embedded::sync::ExFat` reads exFAT. Both have an
`r#async` twin for single-threaded executors such as Embassy. They are
built on the `hadris-fat-raw` device primitives rather than on `FatFs`,
keep one 512-byte block buffer and a fixed number of file slots, and need
neither `std` nor `alloc`.

```toml
[dependencies]
hadris-fat = { version = "3.0.0-rc.1", default-features = false, features = ["sync", "write"] }
hadris-fs = { version = "3.0.0-rc.1", default-features = false }
hadris-storage = { version = "3.0.0-rc.1", default-features = false, features = ["sync"] }
```

`write` adds `format`; drop it if the firmware only mounts cards that are
already formatted. Use `async` in place of `sync` for the async API.

## Provide a device

The embedded API takes a device whose blocks are 512 bytes. Sync firmware
implements `hadris_storage::sync::BlockDevice`; async firmware implements
`hadris_storage::local::BlockDevice`, whose futures need not be `Send`.
[Adapt a custom device](./custom-io.md) shows the trait. An SD card driver
maps `read_blocks` and `write_blocks` onto its block commands.

## Mount, write and read

```rust,ignore
use core::ops::ControlFlow;
use hadris_fat::embedded::{MountToken, Options, sync::Fat};
use hadris_fs::{DirCursor, OpenOptions};

let mut token = MountToken::new();
let mut fat: Fat<_> = Fat::mount_with(card, &mut token, Options::new())?;
let root = fat.root();
let logs = fat.create_dir_all(root, "data/logs")?;
let log = fat.open(logs, "boot.txt", OpenOptions::new().write().create().append())?;
fat.write(&log, b"booted\n")?;
fat.close(log)?;
fat.list(logs, DirCursor::START, |entry| {
    // entry.chars() and entry.name_utf16() borrow the name for this call only.
    ControlFlow::Continue(())
})?;
let card = fat.unmount()?;
```

- A `Dir` is a `Copy` handle. Names are passed one component per call;
  `create_dir_all` is the one method that takes a path.
- A `File` is a slot index that `close` consumes. `Fat<'mount, D, FILES>` has
  `FILES` slots, 4 by default. A separate `MountToken` identifies each mount and stays borrowed while the
  volume or any of its files remain usable. Foreign files fail with
  `InvalidHandle`, even for identical images. A dropped `File` keeps its slot until
  `unmount`, so close files you are done with.
- `list` lends each entry to a callback, so no name is kept in the driver.
  `Entry::node` with `open_node` opens a listed file without a second
  lookup.
- `Options` sets the clock (`with_clock(fn() -> DateTime)`), the UTC
  offset, the code page (CP437 by default), read-only mounting and the
  name fold.

Names fold ASCII case by default, so no Unicode case tables are linked.
`Options::new().with_fold(hadris_fat_raw::fold_unicode)` compares names as
Windows and `FatFs` do, for about 2 KB of flash.

`ExFat::mount(dev, &mut token)` works the same way for reading: `open_dir`, `list`,
`open`, `open_node`, `read`, `seek`, `close`, `metadata`, `label` and
`stats`. It never writes. It is a separate type, so FAT-only firmware does
not link it.

## Power loss and cancelled futures

Writes follow the same ordering as `FatFs`: clusters are marked in the FAT
before anything points to them, long names go before their short entry, and
the active FAT copy is written first. A power cut or a dropped async future
leaves at worst lost clusters, which the next writing call or `sync` on the
same `Fat` frees. A file's size reaches its entry at `flush`, `close`,
`sync` and `unmount`, so call `flush` after a record that must survive a
reset.

## Budget

The [`examples/firmware`](https://github.com/hxyulin/hadris/tree/next/examples/firmware)
package builds firmware-shaped sessions for three targets, and CI measures
them with `scripts/firmware-size.py` at `opt-level = "s"` with fat LTO:

- `fat-log`: a data logger. It creates a directory and a short-named file,
  appends, lists and reads back.
- `fat`: every kind of call. It adds long names, nested directories,
  `rename` and `remove_dir_all`.
- `fat-unicode`: `fat` with `fold_unicode`.
- `fat-async`: `fat` on the async API, polled by a minimal executor.
- `exfat`: the exFAT reader. It reads the label and free space, lists the
  root and reads a file.

Measured on 2026-09-26 with `nightly-2026-09-04`. Sizes are in bytes and
exclude the device driver:

| Target | Session | Flash | Driver state | Mount stack | Worst stack |
|---|---|---|---|---|---|
| thumbv6m-none-eabi (Cortex-M0+) | fat-log | 40268 | 936 | 1944 | 4632 |
| | fat | 45216 | 936 | 1944 | 4608 |
| | fat-unicode | 47288 | 936 | 1944 | 4608 |
| | fat-async | 68728 | 936 | - | 11432 |
| | exfat | 13828 | 1016 | 1952 | 4440 |
| thumbv7em-none-eabihf (Cortex-M4F, M7) | fat-log | 39940 | 936 | 1880 | 4408 |
| | fat | 44824 | 936 | 1880 | 4384 |
| | fat-unicode | 46908 | 936 | 1880 | 4384 |
| | fat-async | 63848 | 936 | - | 10720 |
| | exfat | 13680 | 1016 | 1864 | 4312 |
| riscv32imc-unknown-none-elf (ESP32-C3 class) | fat-log | 46624 | 936 | 1856 | 4288 |
| | fat | 53530 | 936 | 1856 | 4304 |
| | fat-unicode | 55648 | 936 | 1856 | 4304 |
| | fat-async | 73748 | 936 | - | 10624 |
| | exfat | 15994 | 1016 | 1856 | 4288 |

- **Flash** is text, rodata and data of the whole image, which is the
  session and the driver.
- **Driver state** is `size_of::<Fat<'_, D>>()` or `size_of::<ExFat<'_, D>>()`
  with 4 file slots. Keep it in a `static` or on the stack; the driver has
  no other RAM and no statics of its own.
- **Mount stack** is the deepest stack below `mount`, without the driver
  state it returns.
- **Worst stack** is the deepest stack from reset through the session. It
  includes the driver state and the session's own buffers, and for
  `fat-async` the pinned future of the whole session.

The stack figures follow the direct calls in the binary. Calls through a
function pointer, such as the clock and the fold, and compiler builtins
such as `memcpy` count as 0.

What fits, going by the thumbv6m and thumbv7em rows:

| Part | Fits |
|---|---|
| Cortex-M0+, 32 KB flash | The exFAT reader, not FAT |
| Cortex-M0+, 64 KB flash, 8 KB RAM | Sync FAT read and write, with about 3.5 KB of RAM left beside the worst stack |
| Cortex-M4F, 256 KB flash, 64 KB RAM | Sync or async FAT and the exFAT reader together |

CI fails when a mount stack reaches 2 KB or driver state reaches 2 KB
(NF-STACK-01 in `docs/v3/actions.md`), when a Hadris stack frame exceeds
1 KB (NF-STACK-02), or when the `fat-log` flash on thumbv7em grows past its
44 KB ceiling. The 20 KB flash target of NF-FLASH-01 is a goal for 3.x,
not a 3.0 requirement.

To measure locally:

```bash
rustup toolchain install nightly-2026-09-04 --component llvm-tools \
  --target thumbv6m-none-eabi,thumbv7em-none-eabihf,riscv32imc-unknown-none-elf
RUSTUP_TOOLCHAIN=nightly-2026-09-04 scripts/firmware-size.py --check
```

The sessions also run on the host against a freshly formatted memory
device: `cargo run -p hadris-example-firmware --bin fat-log`.

# One async ISO driver

Storage's `async` feature exposes `hadris_storage::async_::BlockDevice` for both
local and Send devices. Implement its poll hooks and operation-owned
`State: Default + Unpin` once. `SendBlockDevice` is derived automatically when
both the device and its state are Send. The [device migration guide](async-devices.md)
explains backend hooks, cancellation, stream adapters and legacy implementations.

ISO uses one `hadris_iso::async_::IsoFs<D>` type. `r#async` remains an alias;
storage's `local` namespace also aliases its common `async_` API. Partitions,
caches and read-ahead adapters accept common devices without a `Local` wrapper.

## Mount and use volumes

Enable `hadris-iso/async` for both local and Send callers:

```rust,ignore
use hadris_iso::async_::IsoFs;
use hadris_fs::MountOptions;

let fs = IsoFs::mount(device, MountOptions::new()).await?;
let volume = hadris_fs::local::Volume::new(fs);
```

For a device and state that are Send, the same mount can be used with
`hadris_fs::async_::Volume`. Generic callers requiring Send futures should use
`hadris_fs::async_::FileSystem`; `hadris_fs::local::FileSystem` accepts either
kind of device. A Send filesystem value alone does not guarantee Send I/O
futures. Local futures may retain `Rc` state across suspension.

Volumes require `alloc` and pointer-sized atomics. The reader also supports
allocation-free `no_std` configurations. The `async-local` feature selects the
local contract without enabling Send volume support.

## Writing and sessions

ISO writers and sessions use the same common device contract and support local
and Send devices. Enable `alloc` (or `std`) for writers and sessions. Their async APIs are available
under `async_` with `r#async` aliases; sync APIs retain their existing namespace.
Regression tests exercise local writes, appended sessions, rewrites and exports
while preserving GPT and El Torito data.

FAT/exFAT, UDF, NTFS, APFS, partition tables and umbrella detection/opening use
the same unified storage contract. CPIO adopts the namespace convention while
retaining its Send byte-stream API. Lazy tree content sources still require
Send and Sync; local volumes do not expose lazy `read_tree`.

# One async ISO driver

Storage's `async` feature exposes `hadris_storage::async_::BlockDevice` for both
non-`Send` and `Send` devices. Implement its poll hooks and operation-owned
`State: Default + Unpin` once. `SendBlockDevice` is derived automatically when
both the device and its state are Send.
[Implement an asynchronous block device](../website/docs/guides/async-devices.md)
explains backend hooks, cancellation and stream adapters.

ISO uses one `hadris_iso::async_::IsoFs<D>` type. Partitions,
caches and read-ahead adapters accept every device without a wrapper.

## Mount and use volumes

Enable `hadris-iso/async`:

```rust,ignore
use hadris_iso::async_::IsoFs;
use hadris_fs::MountOptions;

let fs = IsoFs::mount(device, MountOptions::new()).await?;
let volume = hadris_fs::async_::Volume::new(fs);
```

`IsoFs` implements `hadris_fs::async_::FileSystem` for every device, so the
volume works on single-threaded executors with devices that keep `Rc` state
across suspension. For a device and state that are Send, the futures of that
same volume are `Send`.

Code generic over the filesystem cannot prove that, so it bounds on
`hadris_fs::async_::SendFileSystem`, which `IsoFs` implements for
`SendBlockDevice` devices, and wraps it in `hadris_fs::async_::SendVolume`:

```rust,ignore
use hadris_fs::async_::{SendFileSystem, SendVolume, Sendable};

fn share<F: SendFileSystem + 'static>(fs: F) -> SendVolume<F> {
    SendVolume::new(Sendable::new(fs))
}
```

Async `read_tree` takes a `SendVolume`, so the lazy content it produces is
`Send` for every writer.

Volumes require `alloc` and pointer-sized atomics. The reader also supports
allocation-free `no_std` configurations.

## Writing and sessions

ISO writers and sessions use the same common device contract and support
non-`Send` and Send devices. They need the `write` feature, which is on by
default. Their async APIs are available under `async_`; sync APIs retain their
existing namespace.
Regression tests exercise non-`Send` writes, appended sessions, rewrites and
exports while preserving GPT and El Torito data.

FAT/exFAT, UDF, NTFS, APFS, partition tables and umbrella detection/opening use
the same unified storage contract. CPIO adopts the namespace convention and
accepts non-`Send` byte streams.

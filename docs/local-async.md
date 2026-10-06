# Local asynchronous filesystem access

Issue [#267](https://github.com/hxyulin/hadris/issues/267) identified a V2
migration gap: a filesystem can need an allocator while its device and I/O
futures cannot cross threads. The `async-local` feature enables `local`
modules in `hadris-fs`, `hadris-fat`, `hadris-iso` and `hadris-udf` independently
of their existing `async` feature. This feature is under development on `main`
and is not in the published RC1 packages.

| Module | Operations | Allocation |
|---|---|---|
| `hadris_fs::local` | `FileSystem`, path resolution, `Walk`, contract checks | None |
| `hadris_fs::local` | `Volume`, file/directory handles, `copy_tree`, `ContentReader` | `alloc` |
| `hadris_fat::local` | FAT driver reads and writes | `alloc` |
| `hadris_fat::exfat::local` | exFAT driver reads and writes | `alloc` |
| FAT/exFAT local modules | Unmounted checkers; formatting with `write` | None |
| `hadris_iso::local` | ISO reader | None |
| `hadris_udf::local` | UDF reader | None |

The trait and driver implementations compile from the same source as the sync
and Send async modes. They use `hadris_io::local` and
`hadris_storage::local::BlockDevice`. No executor or boxed device future is
introduced. A local operation may suspend, so its future still needs polling
by a single-threaded executor.

```rust,ignore
use hadris_fat::local::FatFs;
use hadris_fs::local::Volume;
use hadris_fs::{MountOptions, OpenOptions};
use hadris_io::local::Read;

let fs = FatFs::mount(device, MountOptions::new()).await?;
let volume = Volume::new(fs);
let mut file = volume.open("/readme.txt", OpenOptions::new().read()).await?;
let mut buffer = [0u8; 128];
let count = file.read(&mut buffer).await?;
file.close().await?;
```

The runnable [example](../examples/local-async/src/main.rs) creates temporary
in-memory images. Its device and suspending I/O futures retain `Rc` state;
compile-time assertions reject Send and Sync for the device and local ISO
driver. Tests cover writable FAT/exFAT, Unicode names, ISO/UDF read-only
contracts, shared volumes, cancellation during open and the unchanged Send
future guarantee of the existing async tier:

```bash
cargo run -p hadris-example-local-async
cargo test -p hadris-example-local-async
```

## Boundaries

- `Volume` retains the existing async mutex, `Arc` sharing and queued cleanup.
  It needs `alloc` and pointer-sized atomics. Its local instantiation cannot
  cross threads when its driver cannot. `FileSystem` and optical readers do
  not require atomics or allocation.
- Device errors still satisfy `core::error::Error + Send + Sync + 'static`.
  Supporting a non-Send device does not relax the shared error contract.
- Local `read_tree` is absent. Existing lazy `Tree` content uses Send + Sync
  source objects and Send boxed futures; a local source needs a separate design
  that preserves the Send guarantee of existing `Tree` content.
- ISO/UDF local image writers and ISO sessions are absent. Ordinary file
  reads use the full shared filesystem interface. FAT/exFAT tree writers can
  consume byte content; lazy content from existing Send async volumes remains
  subject to its existing mode restrictions.
- Written handles must be closed and the driver synced. Dropping a local
  handle queues the same cleanup as the existing async API; it does not make
  writes durable. Cancellation can leave partial mutations under the existing
  filesystem contract.

## Naming

The issue fix preserves `sync`, `r#async` and the existing lower-level `local`
convention. A separate follow-up will consider `async_` as the canonical Send
async module name, retaining `r#async` as a compatibility alias. Module aliases
would refer to the same types and add no additional generated implementation.

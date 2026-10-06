# One async ISO reader

Storage's `async` feature enables `hadris_storage::async_::BlockDevice` and
`SendBlockDevice` together. The common contract permits non-Send futures;
Send implementations satisfy it automatically. The stronger contract is the
existing Send trait under its canonical name, so existing implementations
remain compatible.

The ISO reader now uses one `hadris_iso::async_::IsoFs<D>` type for Send and
local devices. The parser and reader algorithms remain in `image.rs` and are
compiled once for async use. `hadris_iso::r#async::IsoFs` reexports that type,
so existing mounts, generic filesystem code, volumes and sessions continue
using the same public type.

## Send devices

Enable `hadris-iso/async`. Existing `hadris_storage::r#async::BlockDevice`
implementations, including borrowed devices, partitions, caches and read-ahead
adapters, work directly:

```rust,ignore
use hadris_iso::async_::IsoFs;
use hadris_fs::MountOptions;

let fs = IsoFs::mount(device, MountOptions::new()).await?;
let volume = hadris_fs::r#async::Volume::new(fs);
```

Both inherent reader operations and operations called through the existing
Send filesystem trait return Send futures for these devices. A generic caller
that needs Send futures continues to bound its filesystem by
`hadris_fs::r#async::FileSystem`. Merely requiring the filesystem value to be
`Send` does not express that its I/O futures are Send.

## Local devices

`hadris-iso/async` already enables the common reader for both kinds of devices.
Use `async-local` instead when only the reader and local filesystem contract
are needed, without Send volumes, writers or sessions. A new local device implements
`hadris_storage::async_::BlockDevice` directly and mounts with the same call:

```rust,ignore
let fs = IsoFs::mount(device, MountOptions::new()).await?;
```

Existing `hadris_storage::local::BlockDevice` implementations and adapter chains
remain usable by adapting the outermost device:

```rust,ignore
use hadris_iso::async_::IsoFs;
use hadris_storage::{Partition, local::Cache, async_::Local};
use hadris_fs::MountOptions;

let partition = Partition::new(&mut device, offset_bytes, length_bytes);
let device = Cache::new(partition, cache_blocks);
let fs = IsoFs::mount(Local::new(device), MountOptions::new()).await?;
let volume = hadris_fs::local::Volume::new(fs);
```

Local futures can retain `Rc` or other non-Send state across suspension. The
reader implements `hadris_fs::local::FileSystem` and works with local volumes,
handles and the contract kit. Volumes need `alloc` and pointer-sized atomics;
the reader itself also works without allocation or `std`. Enable both async
features when an application uses both contracts.

`async_` reexports existing Send adapter types. Their constructors retain
Send device requirements; generalizing those adapters to the common contract
is a follow-up. Legacy local adapters keep their existing namespace.

The explicit `Local` compatibility adapter avoids conflicting blanket implementations:
a device can implement both existing device traits. Build local adapters
before wrapping; `Local::into_inner` returns the original device and its
adapter state. Wrapping a Send device as local selects the weaker contract
and does not promise Send futures to generic callers.

## Scope and remaining distinctions

This is an ISO reader pilot. FAT, UDF and other drivers have not been migrated.
ISO writers and sessions still require the Send device contract and the
`async` feature; `async_` reexports their existing implementations. Sync APIs
remain unchanged.

Separate filesystem traits and volume types still express separate guarantees.
The driver no longer needs a second local type or a second async compilation
of its algorithms. Forwarding implementations expose the same reader through
both contracts without boxing or duplicating the parser. Local volumes do not
provide lazy `read_tree`: shared `Content` currently requires Send and Sync.

The regression suite covers genuinely suspending non-Send I/O, borrowed
partition/cache/read-ahead chains, local volume handles, generic Send callers,
and moving a borrowed Send read future to a scoped thread. Compatibility is
also checked against main with cargo-semver-checks. This change does not claim
a measured throughput or RSS improvement.

# Unified asynchronous devices

The RC2 device API uses one asynchronous implementation for local and Send
operations. `async_` is the canonical namespace; `r#async` remains an alias.
The `sync` namespace keeps its synchronous contract.

## Device implementations

Implement `hadris_storage::async_::BlockDevice` once. It defines geometry,
optional write capability, an associated `State: Default + Unpin`, and poll
hooks for read, write and flush. `cancel` stops an interrupted operation.
Callers continue to use `read_blocks(...).await`, `write_blocks(...).await`
and `flush().await`; the provided methods return concrete operation futures.
A fresh state belongs to each operation, rather than to a second filesystem
type or a boxed future.

`SendBlockDevice` is automatic when the device and its operation state are
Send. A Send device with state containing `Rc` remains a valid local device,
but its operation futures cannot cross threads. Generic code that needs Send
futures must use `D: SendBlockDevice`, rather than only `D: BlockDevice + Send`.
Borrowed devices do not need to be `'static` for scoped-thread execution.

`&mut D`, `Box<D>`, `Partition<D>`, `Cache<D>` and `ReadAhead<D>` forward the
same capability. Memory devices accept local buffers too. Cache and read-ahead
retain their configured bounds; adopting the contract does not require
preloading filesystem metadata or file contents.

## Filesystem callers

FAT/exFAT (including embedded), ISO, UDF, NTFS and APFS use the common device
contract. Partition tables and umbrella detection/opening do too. The same
concrete filesystem type implements `hadris_fs::local::FileSystem` for common
devices and the stronger `hadris_fs::async_::FileSystem` for Send devices.
The `async` feature enables both filesystem contracts.

Use `hadris_fs::local::Volume` for a local device and
`hadris_fs::async_::Volume` when the stronger contract is needed. Local volumes
still require allocation and pointer-sized atomics. The allocation-free
filesystem drivers and embedded FAT APIs remain usable without either.
Enabling the `async` feature does not make a local device Send.

CPIO adopts the namespace convention but consumes byte streams through
`hadris_io`, rather than block devices. Its existing stream contracts remain
Send. Unifying local and Send CPIO streams requires a separate byte-stream
contract migration. Lazy tree-content sources also retain their existing
Send requirements; local output devices work with the supported content
sources, but the device change does not remove those source requirements.

## Polling and cancellation

Poll hooks must stop accessing caller buffers before returning, including
`Poll::Pending`. A hardware request that continues between polls needs owned
stable buffers. Movable operation state must not retain pointers to its own
fields. Buffer safety cannot depend on dropping the future: a caller may
forget it or call the safe hooks directly.

Pending hooks must arrange a wakeup when they can make progress. Hooks must
release resources on terminal completion. Dropping an operation calls
`cancel` when a hook is pending or unwinds. Cancellation may run before the
first hook starts I/O, so it must be a safe no-op when idle and must not panic.
Read/write/flush hooks share one state only for their individual operation.

Write errors and cancellation can leave partial output. Successful flush
establishes the underlying backend's durability guarantee; this API does not
provide transactional rollback. Write-back caches retain interrupted dirty
work for retry, and invalidate metadata before an underlying write can change
its bytes.

## Stream adapters and migration

`async_::StreamDevice` wraps the poll-native `async_::Stream` contract. Its
operation state tracks seek and partial transfer progress safely across polls.
It forwards cancellation to the stream. Growth is controlled by the backend's
explicit capability.

With `sync`, `BlockingStream::new(stream)` adapts an existing synchronous
stream. It blocks the executor during the synchronous call. Use
`BlockingStream::new_growable(stream)` only when the stream supports extending
its current length; writable fixed buffers are not growable. Native
`host::FileDevice` async hooks also perform blocking host I/O during polling.

Older async-trait stream adapters remain in
`hadris_storage::legacy_async` and `legacy_local`. They are compatibility
interfaces, not unified devices. There is no automatic generic conversion
from a borrowed async-trait method to a retained poll operation: that would
require the adapter to store a future borrowing itself or the caller's buffer.
Use a poll-native backend, an explicit blocking bridge where appropriate, or
keep the older stream adapter with code that still uses its legacy contract.

Custom async block-device implementations must replace async method bodies
with poll hooks and owned state. A backend that completes immediately can use
`State = ()` and return `Poll::Ready`; a suspending backend records progress
without retaining caller-buffer access. This is a deliberate RC2 API change.

## Validation

Production tests cover local devices, Send devices with local operation state,
borrowed Send futures executed on scoped threads, read/write round trips,
cache invalidation and retry, partial stream transfers, thread-driven wakeups,
errors and cancellation. ISO sessions and UDF bridge writing are included.
The temporary source-copy experiment has been replaced by these production
tests, and CI runs the affected crates on Rust 1.88 and Rust 1.97.1.

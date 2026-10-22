---
title: Implement an asynchronous block device
---

# Implement an asynchronous block device

This page is the reference for the asynchronous block-device contract. The
device API uses one asynchronous implementation for local and Send
operations in the `async_` namespace. The `sync` namespace keeps its
synchronous contract. For using the async drivers, start with
[asynchronous I/O](./async-io.md).

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
concrete filesystem type implements `hadris_fs::async_::FileSystem` for every
device and `hadris_fs::async_::SendFileSystem` for Send devices.

Use `hadris_fs::async_::Volume` for any device; its futures are `Send` when
the concrete device and its operation state are. Code generic over the
filesystem that spawns work uses `hadris_fs::async_::SendVolume`, a `Volume`
over `Sendable::new(fs)`. Volumes require allocation and pointer-sized atomics.
The allocation-free filesystem drivers and embedded FAT APIs remain usable
without either.

CPIO adopts the namespace convention but consumes byte streams through
`hadris_io`, rather than block devices; those traits accept non-`Send`
streams. Lazy tree-content sources stay `Send`: async `read_tree` takes a
`SendVolume`. Non-`Send` output devices work with every content source.

## Polling and cancellation

Poll hooks must stop accessing caller buffers before returning, including
`Poll::Pending`. A hardware request that continues between polls needs owned
stable buffers. Movable operation state must not retain pointers to its own
fields. Buffer safety cannot depend on dropping the future: a caller may
forget it or call the safe hooks directly.

Read and write hooks reject buffers that are not whole blocks and ranges
beyond the readable or maximum writable block count, including direct hook
calls. Invalid requests must not reach the backend or change cached data.

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

`async_::Stream` is not the byte-stream contract of the format crates. CPIO
and lazy tree content read through `hadris_io::async_::{Read, Write, Seek}`
which use `async fn` and return the bare error.
`Stream` has the block device's shape instead (`Error<Self::Error>`, owned
`State`, `cancel`) and only backs `StreamDevice`. `ByteView` turns a block
device into the `hadris_io` traits; nothing turns those traits into a
`Stream`, so a source that serves both a CPIO reader and a `StreamDevice`
implements both traits.

With `sync`, `BlockingStream::new(stream)` adapts an existing synchronous
stream. It blocks the executor during the synchronous call. Use
`BlockingStream::new_growable(stream)` only when the stream supports extending
its current length; writable fixed buffers are not growable. Native
`host::FileDevice` async hooks also perform blocking host I/O during polling.

The older async-trait stream adapters (`hadris_storage::legacy_async` and
`legacy_local`) were removed before 3.0.0. There is no automatic generic
conversion from a borrowed async-trait method to a retained poll operation:
that would require the adapter to store a future borrowing itself or the
caller's buffer. Use a poll-native backend or an explicit blocking bridge
where appropriate.

Custom async block-device implementations must replace async method bodies
with poll hooks and owned state. A backend that completes immediately can use
`State = ()` and return `Poll::Ready`; a suspending backend records progress
without retaining caller-buffer access. This is a deliberate 3.0 API change.

## Validation

Production tests cover local devices, Send devices with local operation state,
borrowed Send futures executed on scoped threads, read/write round trips,
cache invalidation and retry, partial stream transfers, thread-driven wakeups,
errors and cancellation. ISO sessions and UDF bridge writing are included.
CI runs the affected crates on Rust 1.88 and Rust 1.97.1.

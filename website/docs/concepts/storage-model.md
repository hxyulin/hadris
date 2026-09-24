---
title: Storage and I/O model
---

# Storage and I/O model

Hadris keeps storage access separate from format parsing. A typical operation
passes through these layers:

```text
file, memory, firmware protocol, or device driver
                         |
      hadris-io streams / hadris-storage block devices
                         |
      partition window (hadris-part, Partition)
                         |
       format driver (FatFs, IsoView, UdfFs, ...)
                         |
   hadris-fs node API, path helpers, Volume, handles
```

Each layer adds validation or interpretation without hiding the layer below it.
Applications can use only the pieces they need.

## Byte streams with `hadris-io`

`hadris-io` defines `Read`, `Write` and `Seek` in each mode
(`hadris_io::sync::Read`, `hadris_io::r#async::Read`), each reporting the
implementor's own error through `ErrorType`. It supplies explicit adapters:
`StdIo` for `std::io` types, `ToStd` for the other direction, and
`FromEmbedded` for `embedded-io` devices. Firmware and kernels implement the
same traits for their own device handles.

The CPIO reader and writer work on these streams directly, so they run over
pipes.

## Block geometry with `hadris-storage`

Every filesystem driver reads a `hadris-storage` `BlockDevice`, which reads
and writes whole logical blocks of an explicit size. It does not assume
512-byte sectors. `host::FileDevice` is a host image file or disk device
with 512-byte blocks, `Vec<u8>` is an in-memory image that grows when
written past its end, `MemDevice` wraps fixed bytes in memory, `StreamDevice` turns any seekable stream
into a device with the block size you give it, and `Cache` adds a write-back
block cache. Every block operation returns `hadris_io::Error<E>` over the
device's own error `E`: a device refuses writes with kind `ReadOnly`, and an
adapter refuses a request past its end with kind `InvalidInput` and the
block it concerns. A device that accepts writes says so through
`writable()`, which defaults to false; a driver mounts a device that is not
writable read-only.

The format crates validate their own sector and filesystem geometry on top of
the device's block size.

## Partition boundaries

Partition tables describe bounded regions of a larger disk. Before opening a
filesystem inside a partition, create a checked view (a `Partition` of a
block device) restricted to that partition. This prevents filesystem offsets from escaping into neighboring
partitions and keeps offsets relative to the filesystem start.

`hadris-part` reads, edits and writes MBR, GPT and hybrid tables on a block
device, and its `open` turns a partition into such a slice. `hadris-block` adds
detection on block devices, and opens the FAT, exFAT or NTFS volume a slice
holds, when an application needs both the partition and filesystem layers.

## Format handles

Every driver (`FatFs`, `ExFatFs`, `IsoView`, `UdfFs`, `NtfsFs`) implements
the `hadris-fs` `FsDriver` trait through its own inherent methods, and keeps
a native API for what the trait does not model, such as FAT attributes, Rock
Ridge metadata and UDF descriptors. Category facades detect and open formats,
implement the same trait over whichever driver they opened, and keep that
driver reachable.

## Tiers above the driver

A driver takes `&mut self` and holds no lock. Each layer above it is opt-in
and can do every job:

| Tier | Build it with | Paths and handles |
|---|---|---|
| Raw | `FatFs::open(dev)?` | Node ids through the inherent methods; `DriverExt` path helpers on `&mut self`; `File<&mut FatFs<_>>` borrows the driver |
| Shared | `Volume::new(fs)`, `Volume::spin(fs)`, `Volume::local(fs)` | `PathExt` helpers on `&self`, any number of `File` handles |
| Owned | `Arc::new(Volume::new(fs))` | The shared API; handles that move to other threads or tasks |

`lookup` pins a node and `forget` unpins it. Directory entries are plain
values that borrow nothing, and a handle reads its file by offset, so the
driver keeps no cursor for it. Format-specific calls on a shared volume go through `vol.lock()`.

## Choosing the boundary

| Starting point | Recommended layer |
|---|---|
| A known standalone FAT image | Open it directly with `hadris-fat` |
| An unknown disk image | Detect it with `hadris-block` |
| A filesystem inside GPT or MBR | Create a partition view, then open the leaf filesystem |
| An unknown optical image | Use `hadris-optical` with an open policy |
| A custom firmware device | Implement or adapt `hadris-io` traits |
| A logical-block-native device | Start with `hadris-storage` |

See [Open FAT inside a partition](../guides/open-partitioned-fat.md) for an
end-to-end example of the full stack.

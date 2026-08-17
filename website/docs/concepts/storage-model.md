---
title: Storage and I/O model
---

# Storage and I/O model

Hadris keeps storage access separate from format parsing. A typical operation
passes through these layers:

```text
file, memory, firmware protocol, or block device
                         │
                  hadris-io traits
                         │
          bounded device or partition view
                         │
             filesystem or archive parser
                         │
          directory entry and content reader
```

Each layer adds validation or interpretation without hiding the layer below it.
Applications can use only the pieces they need.

## Byte streams with `hadris-io`

Format crates use Hadris `Read`, `Write`, and `Seek` abstractions instead of
depending directly on `std::io`. The crate supplies sync and async traits and
hosted adapters, while firmware and kernels can implement the same traits for
their own device handles.

This is why the same parser code can run over a host file, a memory cursor, or
a custom device without changing the filesystem API.

## Block geometry with `hadris-storage`

`hadris-storage` adds checked logical-block addressing and block-device
capability traits. It does not assume 512-byte sectors. Use it when an
application naturally addresses storage by logical blocks rather than a raw
byte cursor.

Seekable byte streams and block devices can be adapted at this boundary. The
format crates continue to validate their own sector and filesystem geometry.

## Partition boundaries

Partition tables describe bounded regions of a larger disk. Before opening a
filesystem inside a partition, create a checked view restricted to that
partition. This prevents filesystem offsets from escaping into neighboring
partitions and keeps offsets relative to the filesystem start.

`hadris-part` exposes the concrete MBR and GPT structures. `hadris-block` adds
detection and convenient partition views when an application needs both the
partition and filesystem layers.

## Format handles

Leaf crates such as `hadris-fat`, `hadris-iso`, and `hadris-udf` expose their
complete format-specific handles. Category facades detect and open formats but
return concrete handles rather than a lowest-common-denominator filesystem
trait.

That preserves format-specific features such as FAT attributes, Rock Ridge
metadata, UDF descriptors, and partition GUIDs.

## Entry and content lifetimes

Directory entries are metadata values. Content readers borrow the mounted
filesystem or volume and track their own position through a file's extents or
cluster chain. Keep the volume alive while reading content; clone owned entry
metadata when it must outlive an iterator or intermediate lookup.

## Choosing the boundary

| Starting point | Recommended layer |
|---|---|
| A known standalone FAT image | Open it directly with `hadris-fat` |
| An unknown disk image | Detect it with `hadris-block` |
| A filesystem inside GPT or MBR | Create a partition view, then open the leaf filesystem |
| An unknown optical image | Use `hadris-optical` with an open policy |
| A custom firmware device | Implement or adapt `hadris-io` traits |
| A logical-block-native device | Start with `hadris-storage` |

See [Inspect an MBR or GPT image](../guides/read-partition-table.md) for the
first half of this stack; the task guides also cover opening a filesystem
through a bounded partition view.

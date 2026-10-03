# APFS V3 driver use cases

APFS has a container holding volumes. A container is an inspection object; a
mounted volume is a filesystem. The V3 port keeps those two operations distinct
and measures the driver through the same `FileSystem` contract as other formats.
APFS remains an experimental, read-only format.

The next qualification step is [APFS encryption](apfs-encryption.md), using
native macOS fixtures while keeping Asahi's hardware unlock path distinct.

## Scenarios that shape the API

| User | Scenario | API requirement |
|---|---|---|
| Inspection tool | Enumerate a container's volumes and inspect their superblocks, object maps and space manager | Keep native `Container` inspection, including volume identity and name |
| Desktop application | Open an image, list paths and read or extract files through generic code | `ApfsFs` implements `FileSystem` and works with `Volume`, `Walk` and tree extraction |
| Kernel or FUSE adapter | Look up names, retain inode identifiers, page directory entries and read at an offset | Stable inode-based `NodeId`, correct cursors and metadata, and bounded reads without allocating the whole file |
| Backup or recovery tool | Read sparse files, hard links and symlink targets from an unencrypted volume | Preserve inode identity, fill holes with zeroes and expose symlinks without following them in the driver |
| Multi-volume application | Select one of several volumes explicitly | Select by index, APFS object ID, UUID or name; a default mount refuses ambiguity |
| Async service | Run the same operations through a generic async filesystem | Shared sync/async implementation and `Send` futures; preserve backend error payloads |
| Embedded application with an allocator | Mount without `std` over its own block device | Use `hadris-io` and `hadris-storage` throughout the library |
| Application on damaged or unsupported media | Distinguish corruption, unsupported compression/encryption and device failure | Shared error kinds and backend errors; no successful partial reads presented as complete files |

## Mounting and selection

`ApfsFs::mount(device, options)` mounts the sole volume. A multi-volume container
requires `ApfsFs::mount_volume(device, options, selector)`. The selector names an
index, object ID, UUID or volume name. A selection that does not exist fails;
duplicate names must not silently select one volume. Failed mounts return the
device through `MountError`, as the other V3 drivers do.

Mounting selects the current supported checkpoint and volume view. It does not
merge APFS System and Data volumes into a macOS namespace or implement snapshots,
encryption, compression, writes or recovery. Those limitations remain explicit.

## Shared filesystem behavior

A mounted APFS volume reports itself read-only even if its underlying device can
write. Mutations and writable opens are refused before reaching the device.
Directory listings do not expose `.` or `..`; generic path helpers implement
those components. Directory cursors advance deterministically, hard links refer
to the same inode, metadata reflects on-disk types and times, and reads at or past
EOF follow the shared filesystem contract. Driver reads are indexed by offsets,
rather than by allocating a complete file.

Container inspection and raw parsers stay available for format-specific work.
The driver and container I/O use V3 errors retaining the backend's error; a raw
parser's structural error is translated into the corresponding filesystem kind.

## Integration and evidence

The umbrella recognizes APFS containers and mounts them through `detect`,
following the same single-volume rule. The `unstable-apfs` feature exposes native
inspection and explicit volume selection.
Explicit APFS opening provides selection for multi-volume containers. The unified
CLI uses the driver for ordinary file operations and the container for native
inspection.

Synthetic images exercise the filesystem contract and malformed media, while
native macOS images retain a separate interoperability check. The runnable APFS
use-case example demonstrates the public interfaces through checked outcomes.
Tests cover synchronous and asynchronous behavior, selection, sparse and linked
files, backend failures and refusal to write. Feature checks, API shape/parity
checks and targeted Miri runs accompany the port.

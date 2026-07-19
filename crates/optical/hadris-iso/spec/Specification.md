# ISO-9660 Specification

> Maintainer machine-index for implemented sections:
> [`docs/spec-coverage.md`](../../../docs/spec-coverage.md).

This is an unofficial specification for the ISO-9660 filesystem.

## Table of Contents

- [Introduction](#introduction)
- [Terminology](#terminology)
- [File System Structure](#file-system-structure)
  - [Volume Descriptor](#volume-descriptor)
  - [Path Table](#path-table)
  - [Directory Record](#directory-record)
- [Extensions](#extensions)
- [References](#references)

## Introduction

The ISO-9660 filesystem is a file system standard for CD-ROM and DVD-ROM media. However, it is also commonly used for other types of media,
like bootable USB drives and optical media. It is designed to be read-only, as writing to the filesystem is inefficient and would require in some cases
to rewrite the entire filesystem.

The ISO-9660 filesystem is a hierarchical file system, with a root directory at the top level.

## Terminology

- **Volume Descriptor**: See [Volume Descriptor](#volume-descriptor).
- **Path Table**: See [Path Table](#path-table).
- **Directory Record**: See [Directory Record](#directory-record).
- **File**: A file is a file in the ISO-9660 filesystem, consisting arbitrary data up to 4GB (32-bit length) in size.
- **Directory**: A directory is a directory in the ISO-9660 filesystem, consisting of a sequence of directory records, which can be files or other directories.
- **File Name**: A file name is a sequence of characters that identifies a file or directory in the ISO-9660 filesystem.

Types:
- **u8**: An unsigned 8-bit integer.
- **u16**: An unsigned 16-bit integer.
- **u32**: An unsigned 32-bit integer.
- **u64**: An unsigned 64-bit integer.
- **u16lsb**: An unsigned 16-bit integer, stored in little-endian byte order.
- **u32lsb**: An unsigned 32-bit integer, stored in little-endian byte order.
- **u64lsb**: An unsigned 64-bit integer, stored in little-endian byte order.
- **u16msb**: An unsigned 16-bit integer, stored in big-endian byte order.
- **u32msb**: An unsigned 32-bit integer, stored in big-endian byte order.
- **u64msb**: An unsigned 64-bit integer, stored in big-endian byte order.
- **u16lsbmsb**: Two copies of an unsigned 16-bit integer, stored in little-endian byte order and then in big-endian byte order.
- **u32lsbmsb**: Two copies of an unsigned 32-bit integer, stored in little-endian byte order and then in big-endian byte order.
- **u64lsbmsb**: Two copies of an unsigned 64-bit integer, stored in little-endian byte order and then in big-endian byte order.

## File System Structure

The ISO-9660 filesystem follows a flexible structure, this specification will describe the specifics of the structure, and also best practices for creating and using the filesystem. (From the [hadris](https://github.com/hxyulin/hadris) project)

The basic structure of the ISO-9660 filesystem is as follows:
| Sectors (end exclusive) | Description |
| --- | --- |
| 0-16 | Reserved sectors, used by extensions, see [Extensions](#extensions) |
| 16.. | Volume descriptors, see [Volume Descriptor](#volume-descriptor), there can be an arbitrary number of volume descriptors |
| P1-P2 | Path table, see [Path Table](#path-table) |
| D1-D2 | Directory records, see [Directory Record](#directory-record) |
| F1-F2 | Files, see [File](#file) |

This table only shows what is required to be inside the for the Path table, Directory records, and Files, the placement is not specified in the standard.
However, it is recommended that each of the structures are contiguous in memory, to allow for faster access.

Furthermore, when implementing a ISO-9660 writer, it is recommended to use the following structure:

| Sectors (end exclusive) | Description |
| --- | --- |
| 0-16 | Reserved sectors |
| 16..V | Volume descriptors |
| V..F | Files |
| F..D | Directory records |
| D..P | Path table |

This structures allow for the directories to be easier to write, as all the files are already written. Additionally, directories should be written in an order, so that the nested entries (files/directories) are written before the parents (depth-first sorting). More details for the implementation in the respective sections.

### Volume Descriptor

A volume descriptor is 2048 byte structure, which contains information about the filesystem, such as the volume identifier, the volume set identifier, the publisher identifier, and the data preparer identifier.

There are many different volume descriptors, but they all follow the same header:

| Offset | Size | Type | Description |
| --- | --- | --- | --- |
| 0 | 1 | u8 | Descriptor type, see [Volume Descriptor Type](#volume-descriptor-type) |
| 1 | 5 | u8\[5\] | Standard identifier, see [Standard Identifier](#standard-identifier) |
| 6 | 1 | u8 | Version, currently always 1 |

After the header, the volume descriptor they have different structures, but the size is always padded to 2048 bytes.

Each ISO-9660 image contains a list of volume descriptors starting at LBA 16.
The first descriptor must be the primary volume descriptor, which is the only descriptor that is required to be present.
There can be any number of other volume descriptors, which can be used to store additional information about the filesystem, and ends with an Volume Set Terminator descriptor, padded to the entire sector.

The following table describes the structure of the primary volume descriptor
(ECMA-119 8.4; `@hadris-spec ECMA-119:8.4`, implemented by `PrimaryVolumeDescriptor`):

| Offset | Size | Type | Description |
| --- | --- | --- | --- |
| 0 | 1 | u8 | Descriptor type (1) |
| 1 | 5 | u8[5] | Standard identifier ("CD001") |
| 6 | 1 | u8 | Version (1) |
| 8 | 32 | strA | System identifier |
| 40 | 32 | strD | Volume identifier |
| 80 | 8 | u32lsbmsb | Volume space size (logical blocks) |
| 120 | 4 | u16lsbmsb | Volume set size |
| 124 | 4 | u16lsbmsb | Volume sequence number |
| 128 | 4 | u16lsbmsb | Logical block size (bytes; 2048) |
| 132 | 8 | u32lsbmsb | Path table size (bytes) |
| 140 | 4 | u32lsb | Location of Type-L path table |
| 144 | 4 | u32lsb | Location of optional Type-L path table |
| 148 | 4 | u32msb | Location of Type-M path table |
| 152 | 4 | u32msb | Location of optional Type-M path table |
| 156 | 34 | — | Root directory record (see [Directory Record](#directory-record)) |
| 190 | 128 | strD | Volume set identifier |
| 318 | 128 | strA | Publisher identifier |
| 446 | 128 | strA | Data preparer identifier |
| 574 | 128 | strA | Application identifier |
| 702 | 37 | strD | Copyright file identifier |
| 739 | 37 | strD | Abstract file identifier |
| 776 | 37 | strD | Bibliographic file identifier |
| 813 | 17 | dec-datetime | Volume creation date/time |
| 830 | 17 | dec-datetime | Volume modification date/time |
| 847 | 17 | dec-datetime | Volume expiration date/time |
| 864 | 17 | dec-datetime | Volume effective date/time |
| 881 | 1 | u8 | File structure version (1) |
| 883 | 512 | — | Application use |

The **Supplementary Volume Descriptor** (ECMA-119 8.5; `@hadris-spec ECMA-119:8.5`,
`SupplementaryVolumeDescriptor`) shares this layout; Hadris uses it for the Joliet
namespace, distinguished by an escape sequence in the "Escape Sequences" field
(offset 88) selecting UCS-2 Level 1/2/3.

The **Boot Record Volume Descriptor** (ECMA-119 8.2; `@hadris-spec ECMA-119:8.2`,
`BootRecordVolumeDescriptor`) carries the boot system identifier
`"EL TORITO SPECIFICATION"` and a pointer to the El Torito boot catalog.

The **Volume Descriptor Set Terminator** (ECMA-119 8.3; `@hadris-spec ECMA-119:8.3`,
`VolumeDescriptorSetTerminator`) is a header-only descriptor (type `0xFF`) padded to
2048 bytes that ends the descriptor sequence.

#### Volume Descriptor Type

The volume descriptor type is an enum, which can be one of the following values:

| Value | Description |
| --- | --- |
| 0x00 | Boot record |
| 0x01 | Primary volume descriptor |
| 0x02 | Supplementary volume descriptor |
| 0x03 | Volume partition descriptor |
| 0xFF | Volume set terminator |

#### Standard Identifier

The standard identifier is a 5-byte ASCII string, which is used to identify the standard that the volume descriptor follows.
It is the string "CD001".

### Path Table

The path table (ECMA-119 9.4; `@hadris-spec ECMA-119:9.4`, implemented by
`PathTableEntryHeader` / `PathTableEntry`) is a flat, breadth-first index of every
directory in the image, allowing a reader to locate a directory without walking the
tree. Each record is:

| Offset | Size | Type | Description |
| --- | --- | --- | --- |
| 0 | 1 | u8 | Length of directory identifier (LEN_DI) |
| 1 | 1 | u8 | Extended attribute record length |
| 2 | 4 | u32 | Location of extent (logical block) |
| 6 | 2 | u16 | Directory number of parent |
| 8 | LEN_DI | strD | Directory identifier |
| 8+LEN_DI | 0/1 | u8 | Padding to an even boundary |

Two copies are written: the **Type-L** table stores the extent location little-endian,
the **Type-M** table big-endian. Hadris writes both; the optional secondary path-table
pointers in the PVD are left zero.

### Directory Record

A directory record (ECMA-119 9.1; `@hadris-spec ECMA-119:9.1`, implemented by
`DirectoryRecordHeader` / `DirectoryRecord`) describes one file or directory:

| Offset | Size | Type | Description |
| --- | --- | --- | --- |
| 0 | 1 | u8 | Length of directory record (LEN_DR) |
| 1 | 1 | u8 | Extended attribute record length |
| 2 | 8 | u32lsbmsb | Location of extent (logical block) |
| 10 | 8 | u32lsbmsb | Data length (bytes) |
| 18 | 7 | datetime | Recording date and time |
| 25 | 1 | u8 | File flags (see below) |
| 26 | 1 | u8 | File unit size (interleaved mode) |
| 27 | 1 | u8 | Interleave gap size |
| 28 | 4 | u16lsbmsb | Volume sequence number |
| 32 | 1 | u8 | Length of file identifier (LEN_FI) |
| 33 | LEN_FI | strD | File identifier |
| 33+LEN_FI | 0/1 | u8 | Padding to an even boundary |
| … | … | — | System use area (SUSP / Rock Ridge) |

File flags (offset 25): bit 0 Hidden, bit 1 Directory, bit 2 Associated file,
bit 3 Record format in EAR, bit 4 Permissions in EAR, bit 7 Not-final (multi-extent).
The special identifiers `0x00` ("." — self) and `0x01` (".." — parent) are the first
two records of every directory. Hadris writes records in ascending File Identifier
order (ECMA-119 9.3).

## Extensions

Hadris implements the following on top of base ISO 9660. Each is indexed by its
`@hadris-spec` id in [`docs/spec-coverage.md`](../../../docs/spec-coverage.md).

- **Joliet** — a Supplementary Volume Descriptor whose names are UCS-2 (BMP only),
  giving long, case-preserving, Unicode filenames. `@hadris-spec ECMA-119:8.5`.
- **Rock Ridge / SUSP (RRIP)** — System-use entries in each directory record carrying
  POSIX metadata: `PX` (mode/uid/gid/inode), `NM` (long name), `SL` (symlink),
  `TF` (timestamps, incl. creation), `CL`/`PL`/`RE` (deep-directory relocation),
  `SP`/`CE`/`ER`/`ST` (SUSP framing).
- **El Torito** — a boot catalog referenced by the Boot Record Volume Descriptor:
  a validation entry, a default/initial entry, and optional platform section
  headers/entries. Media emulation types: none, 1.2/1.44/2.88 MB floppy, hard disk.
  `@hadris-spec El-Torito:validation`, `El-Torito:section-header`,
  `El-Torito:section-entry`.
- **Hybrid MBR/GPT** — a partition table embedded so the image also boots as a USB
  disk (via `hadris-part`).

## References

Boot Info Table
https://dev.lovelyhq.com/libburnia/libisofs/raw/branch/master/doc/boot_sectors.txt

ISO9660 File System
https://wiki.osdev.org/ISO_9660

El-Torito File System
https://pdos.csail.mit.edu/6.828/2018/readings/boot-cdrom.pdf

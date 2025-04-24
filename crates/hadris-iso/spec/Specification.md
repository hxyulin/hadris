# ISO-9660 Specification

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

The following table describes the structure of the primary volume descriptor:
| Offset | Size | Type | Description |
| --- | --- | --- | --- |
| 0 | 1 | u8 | Descriptor type |

(WIP)

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

## References

Boot Info Table
https://dev.lovelyhq.com/libburnia/libisofs/raw/branch/master/doc/boot_sectors.txt

ISO9660 File System
https://wiki.osdev.org/ISO_9660

El-Torito File System
https://pdos.csail.mit.edu/6.828/2018/readings/boot-cdrom.pdf

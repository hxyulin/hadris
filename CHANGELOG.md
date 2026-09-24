# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each published package owns its version and may be released independently.

## [Unreleased]

### Added

- **Examples (V3):** A `volume-list` example detects a FAT12/16/32, exFAT
  or NTFS image, opens it with `hadris-block`'s `OpenVolume` and prints its
  tree through one function generic over `FsDriver`. `examples/README.md`
  lists the crate examples, and the detect-and-open guide links to it.
- **hadris-common (V3):** `U16`, `U32` and `U64` have inherent `new`,
  `get` and `set`, so the `raw` layouts of `hadris-fat` and its exFAT
  module can be read and built without importing the `Endian` trait of
  this internal crate.
- **hadris-fat (V3):** `raw` holds the directory entry layouts:
  `RawDirEntry` (short entries, with `lfn_checksum`) and `RawLfnEntry`,
  with the `DIR_Attr`, `DIR_NTRes` and `LDIR_Ord` bits and the end, free
  and `0x05` name markers. The driver decodes and encodes entries through
  them.
- **hadris-fat (V3):** exFAT is stable. `ExFatFs<D, T, C>` in
  `hadris_fat::exfat::{sync, r#async, async_send}`, a sibling of `FatFs`,
  replaces the preview. It needs no allocator, implements the writable
  `FsDriver` through `impl_fs_driver!` (with `parent`, `open_node`,
  `close_node` and `publish_node`) and passes the contract kit in all three
  modes. It reads contiguous and chained allocations, fragmented allocation
  bitmaps and up-case tables, entry sets that cross clusters and benign
  secondary entries (kept across renames), and writes FAT chains: create,
  remove, rename with `NO_REPLACE`, `write_at`, `set_len` (reads past
  `ValidDataLength` return zeros), attributes and the four times, directory
  growth, `label` and `set_label`, `VolumeDirty` and `PercentInUse`. Mounts
  check the boot checksum and fall back to a valid backup boot region
  read-only, and mount read-only when the up-case table fails its checksum.
  TexFAT
  volumes with two FATs are mounted through `ActiveFat` and written with
  both FATs and both bitmaps kept equal. `format` takes
  `exfat::FormatOptions` (`with_label`, `with_volume_id`,
  `with_sector_size`, `with_cluster_size`, `with_alignment`,
  `with_partition_offset`, `with_fat_count`, `with_clock`) and lays out
  volumes like `mkfs.exfat`; `check` and `check_with` report
  `exfat::Finding`s without an allocator. `exfat::raw` holds the boot
  sector, entry layouts and constants. Qualified by the conformance suite
  against exfatprogs, macOS `newfs_exfat`/`fsck_exfat` and the macOS
  kernel driver.
- **hadris-block (V3):** `OpenVolume` opens exFAT as `ExFatFs`, with
  `as_exfat`, `as_exfat_mut` and `into_exfat`.
- **hadris-io (V3):** `SeekFrom` is a Hadris `#[non_exhaustive]` enum with
  `resolve(current, len)`, converting to and from `std::io::SeekFrom` with
  `std` and the `embedded-io` types with the new `embedded-io` feature.

- **hadris-ntfs (V3):** Rewritten on `hadris-storage` block devices, with
  the same API in `sync`, `r#async` and `async_send` (new `async-send`
  feature). `NtfsFs::open` reads a volume without an allocator: records,
  index blocks and a page cache of `$UpCase` are fixed buffers, and MFT and
  index records of up to 4096 bytes on device blocks of up to 4096 bytes
  are supported. `NtfsFs` implements the read-only `FsDriver` through
  `impl_fs_driver!` (with `parent`), so the `hadris-fs` path helpers,
  `Volume`, handles and `extract_to_host` work on it, and passes
  `contract::check_read_only`. Node ids are file references (the record
  number with the sequence number in the top 16 bits), so hard links share
  one id and no node table is needed. `$ATTRIBUTE_LIST` entries are
  followed into extension records for streams, names and index roots, and
  for a `$MFT` of up to 32 extents. Update sequence arrays are applied in
  512-byte strides whatever the sector size. Metadata has the four times
  and DOS attributes of `$STANDARD_INFORMATION` and the number of
  non-DOS names. Listings leave out DOS aliases and the metadata files
  (records below 16), which `lookup` still finds; an exact listed name
  wins over a case-folded match, and names with unpaired surrogates show
  U+FFFD. `streams` and `read_stream_at` read named data streams, and
  `label`, `volume_serial`, `cluster_size`, `sector_size`,
  `total_sectors`, `mft_record_size` and `index_record_size` are native;
  `stats` counts free clusters from `$Bitmap`. `raw` holds `BootSector`
  and the attribute, flag, namespace and record codes. `Error<E>` carries
  an `ErrorKind`, a `Detail` and the device error, converts into
  `hadris_fs::Error<E>`, `AnyError` and `std::io::Error`, and opening
  fails with `MountError`. Compressed and encrypted streams fail with
  `Unsupported`. NTFS stays a preview.
- **hadris-block (V3):** `detect` recognizes NTFS (`BlockFormat::Ntfs`),
  checking the NTFS and exFAT OEM identifiers before partition entries.
  `OpenVolume` opens FAT12/16/32 and NTFS in every build and implements
  `FsDriver` by delegation (with `parent`, `open_node`, `close_node` and
  `publish_node`); NTFS answers writes with `ReadOnly`, and the contract
  kit passes through it. `open_detected` takes a `BlockFormat`, and
  `format` returns one. The `unstable-ntfs` feature re-exports
  `hadris-ntfs` as `ntfs` and adds `as_ntfs`, `as_ntfs_mut` and
  `into_ntfs`. `Error` and `OpenError` convert into `AnyError` with
  `alloc`.
- **hadris-optical (V3):** `OpenOpticalImage` owns a block device, opens
  UDF as `UdfFs` or ISO 9660 as an `IsoView` of the preferred namespace,
  and implements the read-only `FsDriver` by delegation (with `parent` and
  `read_link`); `as_iso`, `as_iso_mut`, `into_iso`, `as_udf`, `as_udf_mut`
  and `into_udf` reach the drivers. A failed open gives the device back in
  an `OpenError`. Detection and opening exist in `sync`, `r#async` and
  `async_send` and need no allocator.
- **hadris (V3):** Re-exports `hadris::storage` and `hadris::fs` always,
  and each format crate at a flat path behind a feature of the same name:
  `fat`, `part`, `iso`, `udf`, `cd`, `cpio`, `block`, `optical`, and the
  NTFS preview as `ntfs` behind `unstable-ntfs`.

- **hadris-udf (V3):** Rewritten on `hadris-storage` block devices, with
  the same API in `sync`, `r#async` and `async_send`. `UdfFs::open` reads
  a volume without an allocator: it finds an anchor at block 256, N-256 or
  N-1 for logical blocks of 512 to 4096 bytes, checks the recognition
  sequence, keeps the prevailing descriptors of the main sequence or, when
  it is damaged, the reserve sequence, follows descriptor pointers, and
  maps up to eight type 1 partitions. `UdfFs` implements the read-only
  `FsDriver` through `impl_fs_driver!` (with `parent` and `read_link`), so
  the `hadris-fs` path helpers, `Volume`, handles and `extract_to_host`
  work on it. Node ids are ICB locations, so hard links share one id and
  no node table is needed. File entries and extended file entries are read
  with short, long, extended and embedded allocation descriptors,
  continuation extents and unrecorded extents; metadata has the three
  times (four with extended entries), permissions, owner and link count;
  symlink path components become `/`-joined targets. `volume_id`,
  `logical_volume_id`, `revision`, `block_size`, `partitions`
  (`Partition`), `extents` and `read_bytes` are native, and `raw` holds
  the on-disk layouts with little-endian field types. Virtual, sparable
  and metadata partitions fail with `Unsupported`. `write(dev, &tree,
  &options)` and `plan(&tree, &options)` (`alloc`) lay out a
  `hadris_fs::tree::Tree` as `UdfOptions` says (volume identifier,
  `UdfRevision`, `with_min_blocks`, `Bridge` and an injected `Clock`) and
  return a `Report` of the size, each file's extent, `allocated_end` and
  `Warning`s. Blocks are written once, in ascending order, to any block
  device whose block size divides 2048, without `std`. Symlinks, hard
  links, permissions, owners and times are stored; creation times, DOS
  attributes and device nodes are reported. `Bridge` makes the volume share
  an ISO 9660 image, recording its recognition sequence after the ISO
  descriptors and pointing at `Content::stored` extents. `Error<E>`
  carries an `ErrorKind`, a `Detail` (`#[non_exhaustive]`) and the device
  error, converts into `hadris_fs::Error<E>`, `AnyError` and
  `std::io::Error`, and opening fails with `MountError`.
- **hadris-cd (V3):** `write(dev, &tree, &CdOptions)` and `plan` in
  `sync`, `r#async` and `async_send` write a hybrid image from one
  `hadris_fs::tree::Tree`: the UDF metadata is planned first, the ISO 9660
  image is written after it with `hadris_iso`, and the UDF bridge volume
  points at the file extents of the ISO `Report`, so nothing is read back.
  `CdOptions` holds an `IsoOptions` and a `UdfOptions` (`with_iso`,
  `with_udf`, `with_clock`), both crates are re-exported as `iso` and
  `udf`, and `Report` holds both reports. Symlinks, hard links and
  metadata come from the tree. `Error<E>` keeps the kind and device error
  of the writer that failed with `Detail::Iso` or `Detail::Udf`.
- **hadris-cpio (V3):** Rewritten on `hadris_io` V3 streams, with the same
  API in `sync`, `r#async` and `async_send`. `CpioReader` reads `newc`,
  `newc` with checksums, `odc` and old binary archives in either byte
  order, without an allocator. `next_entry` returns an `Entry` that borrows
  the reader and implements `Read`; data left unread is skipped by the next
  call, with `070702` checksums verified either way. An archive may end at
  an entry boundary without a trailer unless
  `ReaderOptions::with_strict_trailer` is set, and `continue_after_trailer`
  reads concatenated archives such as microcode before an initramfs.
  `CpioWriter` (`alloc`) streams `newc`, `newc` with checksums or `odc`
  entries: `append` takes a `NewEntry` (file `Content`, directory,
  symlink, device, FIFO, socket) with a `SetMetadata`, `append_hard_links`
  writes a hard link group, `write_tree` writes a `hadris_fs::tree::Tree`,
  and `finish` writes the trailer. `write(out, &tree, &CpioOptions)` does
  it in one call and returns a `Report` whose warnings list metadata cpio
  cannot store. Field widths are checked before an entry is written
  (`FileTooLarge`, `NameTooLong`, `LimitExceeded`). `Error<E>` carries an
  `ErrorKind`, a `Detail` and the stream error, and converts into
  `hadris_fs::Error<E>`, `AnyError` and `std::io::Error`. The header
  layouts are in `raw` (`NewcFields`, `NewcHeader`, `OdcFields`,
  `OdcHeader`, `BinaryHeader`).
- **hadris-iso (V3):** Rewritten on `hadris-storage` block devices, with
  the same API in `sync`, `r#async` and `async_send`. `IsoImage::open`
  reads the descriptor set without an allocator and reports the trees the
  image carries (`namespaces`); `view(Namespace)` picks the primary tree,
  Rock Ridge over it, Joliet or the ISO 9660:1999 enhanced tree
  (`Namespace::Preferred` takes the most capable) as an `IsoView`, which
  implements `FsDriver` through `impl_fs_driver!`, so the `hadris-fs`
  path helpers, `Volume`, handles and `extract_to_host` work on it. Node
  ids are directory record offsets and need no node table. Relocated Rock
  Ridge directories appear in their real place, lookups in the primary and
  enhanced trees fall back to ignoring ASCII case, and Joliet trees whose
  `..` points at itself (libisofs) find their parent through the path
  table. Logical blocks of 512 to 2048 bytes and device blocks up to 4096
  bytes work. `IsoView::rock_ridge` (`RockRidgeInfo`), `raw_record`,
  `extents`, `IsoImage::descriptor`, `primary_descriptor`, `read_bytes`
  and `boot_catalog` (`BootCatalog`, `BootCatalogEntry`, `Platform`,
  `Emulation`; `alloc`) expose the rest, and `raw` holds the on-disk
  layouts (descriptors, directory and path table records, El Torito
  entries, boot info tables, SUSP entries). `write(dev, &tree, &options)`
  and `plan(&tree, &options)` (`alloc`) lay out a `hadris_fs::tree::Tree`
  as `IsoOptions` says (`IsoLevel`, `NameCase`, `Charset`,
  `VolumeIdentifiers`, `JolietLevel`, `RockRidge` with `Preserve` and
  `Relocation`, the enhanced tree, `ElTorito` with `BootEntry` images
  named by tree path and `BootInfo` tables, `HybridBoot` MBR, GPT or
  hybrid tables, `min_blocks` and an injected `Clock`) and return a
  `Report` of the size, each file's extents and `Warning`s for what the
  options could not store. The layout is planned without I/O and written
  in ascending block order, so any block device works, without `std`.
  Rock Ridge writes PX, PN, NM, SL, TF, CL, PL and RE, and stores
  symlinks, device nodes and hard links. `Session` reads an image into a
  tree whose files point at their extents and writes it back after
  changes as a new session (`SessionMode::Append`) or in place
  (`SessionMode::Rewrite`, which keeps the boot catalog and moves the
  backup GPT), without copying unchanged files. `Error<E>` carries an
  `ErrorKind`, a `Detail` (`#[non_exhaustive]`) and the device error,
  converts into `hadris_fs::Error<E>`, `AnyError` and `std::io::Error`,
  and opening fails with `MountError`, which gives the device back. Nodes
  inside the volume but past the end of a truncated device are `Corrupt`,
  other bad ids `InvalidHandle`.
- **hadris-fs (V3):** `tree` (`alloc`), the input every V3 writer takes:
  `Tree` holds files, directories, symlinks, device nodes and hard links
  with their `SetMetadata` under `/` paths, children sorted by name
  (`add_file`, `add_dir`, `add_symlink`, `add_device`, `add_hard_link`,
  `set_metadata`, `remove`, `get`, `root`). `Content` is bytes, a blocking
  or async byte source, a host file opened when read (`std`) or extents
  already on the device, and `ContentReader` reads it in each mode.
  `Tree::from_fs` (`std`) imports a host directory, detecting hard links;
  `FromFsOptions::with_on_error(OnError::Warn)` turns unreadable entries
  into `Warning`s. `TreeExt::from_filesystem` builds a tree from any
  mounted filesystem. `Extent` (a byte range) is at the crate root, and
  `contract::check_read_only` runs the read-only rules of the driver
  contract, which the ISO views pass.
- **hadris-part (V3):** The partition tables move onto `hadris-storage`
  block devices and take the block size from the device. `Disk` holds a
  `PartitionTable` (`#[non_exhaustive]`: `Mbr`, `Gpt`, `Hybrid`) and the
  446 bytes of boot code in block 0; it does no I/O. Each mode (`sync`,
  `r#async` and, with the new `async-send` feature, `async_send`) has
  `read(&mut dev)`, `write(&mut dev, &disk)`, `create(&mut dev, &layout)`,
  `open(dev, &partition)`, which returns a `Slice` of the disk, and
  `scan(&mut dev, f)`, which lists partitions through a callback without an
  allocator. `Disk::partitions` yields `Partition` values with `index`,
  `start`, `len`, `end`, `size_bytes` (in the disk's block size), `kind`
  (`PartitionKind::{Mbr(MbrType), Gpt(Guid)}`), `flags`, `attributes`,
  `unique_guid` and `name`. `Disk::runs` gives the table's bytes as runs of
  whole blocks, block 0 last, for writers that are not block devices.
  Feature work: extended and logical MBR partitions read from and written
  as EBR chains (indices from 4, as Linux numbers them); a GPT whose
  primary copy fails validation is read from the backup and the other way
  round, `Gpt::damaged_copy` names the damaged copy, and `write` repairs
  it; GPT names are UTF-16 (`PartitionName`, 36 code units, lossless for
  unpaired surrogates, `Display` replaces them with U+FFFD and `to_str`
  reports them); `Mbr` and `Gpt` edits (`add`, `add_logical`, `remove`,
  `resize`, `set_kind`/`set_type`, `set_flags`, `set_name`,
  `set_unique_guid`, `set_attributes`) check bounds, the 32-bit MBR fields
  and overlap with every other partition and leave the table unchanged when
  they fail. `DiskLayout` (`mbr`, `gpt(disk_guid)`, `hybrid(disk_guid)`)
  places `PartitionSpec`s (`Size::{Blocks, Bytes, KiB, MiB, GiB,
  Remaining}`, `Alignment::{Block, MiB1, Blocks}`, names, flags, explicit
  starts, hybrid mirrors) with `build(block_count, block_size)`; an MBR
  layout with more than four partitions gets an extended partition, and
  unique GUIDs default to ones derived from the disk GUID, so a layout
  always builds the same disk. `HybridMbr` configures a hybrid MBR
  (`with_protective_slot`, `add_mirrored`) and holds no allocation.
  `Error<E>` carries an `ErrorKind` from `hadris-fs`, a `Detail`
  (`#[non_exhaustive]`) naming the structure or partition, and the
  device's own error, and converts into `hadris_fs::Error<E>` and
  `std::io::Error`; edits that touch no device return `TableError`. The
  on-disk layouts (`RawMbr`, `RawMbrEntry`, `Chs`, `RawGptHeader`,
  `RawGptEntry`), the specification constants and the CRC32 live in `raw`.

- **hadris-fs (V3):** The `contract` feature adds a driver test kit:
  `contract::check(&mut fs)` in each mode runs the format-independent rules
  of the `FsDriver` contract (pins, opens and `Busy`, removed pinned nodes,
  `RemoveKind`, `NO_REPLACE` and rename type rules, listed ids, cursor
  ranges and resumption, reads, writes and `set_len`) in a scratch
  directory and returns the first `ContractViolation`. It needs no
  allocator. The hadris-fs test driver and `FatFs` (FAT12, FAT16 and FAT32;
  raw, shared and async) pass it.
- **hadris-fs (V3):** New crate with the shared, mode-independent filesystem
  vocabulary: `NodeId`, `FileType`, byte names (`Name`, `NameBuf`,
  `OwnedName`), `DateTime` with civil-time conversions, `FileTimes`, `Clock`
  (`NoClock`, `SystemClock`), `Mode`, `Attributes`, `Metadata`,
  `SetMetadata`, `Capabilities`, `FsStats`, `ErrorKind`, `DirCursor`,
  `DirEntry`, `OpenOptions`, `RenameFlags` and `NewNode`.
  `Error<E>` is the error of every filesystem operation: an `ErrorKind` plus
  the device's own error `E`, kept without allocation (`device_error`,
  `into_device_error`, `map_device`). It converts from `WriteError<E>`
  (`ReadOnly` becomes `ErrorKind::ReadOnly`) and `NameError`, and with `std`
  into `std::io::Error`, returning an `io::Error` device error as itself.
  `AnyError` (`alloc`) erases the device type for code that mixes devices.
  `FsResult<T, E>` names the result. `MountError<D, E>` is the error of a
  mount or format that takes its device by value: the `Error<E>` and the device given
  back (`kind`, `error`, `device`, `into_error`, `into_device`,
  `into_parts`). `?` converts it into `Error<E>`, `AnyError` or
  `std::io::Error`.
  With the `sync`, `async` and `async-send` features, the driver layer, one
  source generated per mode: `FsDriver` (`&mut self`, for format crates) and
  `FileSystem` (`&self`), `impl_fs_driver!` to implement `FsDriver` from
  inherent methods, `Volume<F, K>` with `StdMutex`, `Spin`, `Local`,
  `AsyncMutex` and (with `embassy-sync`) an async `Local` lock, the
  `Lexical` and `Posix<N>` resolvers and `WithResolver`, the `DriverExt` and
  `PathExt` path helpers, `OpenFile`, and `File<A>`/`Dir<A>` handles over
  `Access` (`&mut D`, `&F`, `Arc`, `Rc`, `Volume`). `DirItem` pairs an entry
  with its name, and `ErrorKind::Symlink` reports `ELOOP`.
  `copy_tree` (`alloc`) copies a file, symlink or directory tree between
  any two filesystems, each side any `Access`, and returns `AnyError`. With
  `std`, the sync API adds `extract_to_host` and `import_from_host`.
  `extract_to_host` rejects entry names that are not one plain host
  component (and, on Windows, device names such as `CON` or `com1.txt`,
  names with `:` or wildcards, and names ending in a dot or space), never
  writes through or replaces an existing host symlink, and replaces an
  existing file with a new one instead of truncating it, so its other hard
  links keep their contents. `import_from_host` copies each directory's
  entries in name byte order, so a host tree always gives the same image.
  `copy_tree` and `extract_to_host` refuse symlink targets longer than 4096
  bytes with `ErrorKind::LimitExceeded` instead of allocating whatever
  length the source reports. `FuseOnError` ends an iterator of
  `Result`s after its first `Err`.
  `NodeTable` maps a driver's `NodeId`s to per-node state with pin counts
  for formats without stable inode numbers. `FixedTable<N>` needs no
  allocator, `HeapTable` (`alloc`) grows, and users can implement their own;
  a full table gives `TableFull`, which converts to
  `ErrorKind::LimitExceeded`. `NameBuf` holds 1024 bytes by default, enough
  for any 255-unit UTF-16 long name.
- **hadris-fat (V3):** `FatFs<D, T = FixedTable<64>>`, the V3 driver for
  FAT12, FAT16 and FAT32 on any `hadris_storage` `BlockDevice`, in the
  `sync`, `r#async` and `async_send` modules. It implements `FsDriver`
  through `impl_fs_driver!`, so `Volume`, the path helpers and the `File` and
  `Dir` handles of `hadris-fs` work on it. This first version reads:
  `lookup` (case-insensitive, by long or short name), `read_dir_entry` with
  resumable cursors, `read_at`, `node_metadata` (times and attributes),
  `parent`, `stats` and `forget`; the write methods return
  `ErrorKind::ReadOnly`. `open` mounts a volume, `into_inner` returns the
  device, and `kind` returns the new `FatKind`. Node ids come from the
  location of the directory entry, and `lookup` and `parent` pin them in the
  node table `T`; a full table gives `ErrorKind::LimitExceeded`. Long names
  are always read, and a leading `0x05` in a short name reads as `0xE5`.
  The driver needs no allocator: its only buffer is one device block of at
  most 4096 bytes, and larger blocks are rejected with
  `ErrorKind::Unsupported`.
- **hadris-fat (V3):** `FatFs` writes. `create` makes files and
  directories (other kinds are `ErrorKind::Unsupported`) with long names and
  generated short names (`~1` to `~4`, then two basis characters, four hex
  digits hashed from the long name and `~1` to `~9`, as on Windows), or a
  short entry alone with the NT case bits
  when the name fits 8.3; `remove` deletes files and empty directories
  (`Busy` while pinned, `DirectoryNotEmpty`); `rename` moves within and
  across directories, keeps the node's id, updates `..` of a moved
  directory, replaces an existing target unless `RenameFlags::NO_REPLACE`
  (the result takes the requested name and case, as on Windows, not the
  target's), and rejects unknown flags with `Unsupported`; `write_at` and `set_len`
  grow, zero-fill and shrink files, freeing clusters; created, renamed and
  resized or written files get the archive attribute; `set_metadata` sets
  attributes and creation, modification and access times (mode and owner
  are ignored); `sync_node` and `sync` write pending sizes and the FAT32
  FSInfo free count, and flush the device. Directories grow past their
  first cluster, and a full FAT12/16 root directory gives
  `ErrorKind::NoSpace`. The size of a pinned file lives in the node table
  until it is synced, so handles share it; such a node stays in the table
  after its last `forget`. A device answering `WriteError::ReadOnly` fails
  the operation with `ErrorKind::ReadOnly`, changes nothing and makes the
  volume read-only, as `capabilities` and `is_read_only` then report.
  Writes are ordered so an interrupted operation or a dropped `async`
  future leaves an `fsck`-repairable volume.
- **hadris-fat (V3):** Clock and code page generics:
  `FatFs<D, T = FixedTable<64>, C: Clock = NoClock, P: CodePage = Ascii>`.
  `MountOptions<T, C, P>` (`new`, `with_read_only`, `with_table`,
  `with_clock`, `with_code_page`) and `FatFs::open_with(dev, options)`
  choose them; `FatFs::open(dev)` keeps the defaults. Both fail with
  `hadris_fs::MountError`, which gives the device back. `clock()` and
  `code_page()` return them. The `CodePage` trait maps short-name bytes
  above `0x7F`; `Ascii` reads them as U+FFFD, and `Cp437` is the IBM PC code
  page. `NoClock` stamps 1980-01-01, and `SystemClock` (`std`) the current
  UTC time.
- **hadris-fat (V3):** `format(dev, FormatOptions) -> FatFs<D,
  FixedTable<64>, C>` in `sync`, `r#async` and `async_send` (the `write`
  feature, no allocator) formats a FAT12, FAT16 or FAT32 volume that fills a
  `BlockDevice`, including a `Slice` of a disk, and mounts it.
  `FormatOptions<C = NoClock>` (`new`, `with_kind`, `with_label`,
  `with_volume_id`, `with_sector_size`, `with_cluster_size`,
  `with_oem_name`, `with_reserved_sectors`, `with_hidden_sectors`,
  `with_fat_count`, `with_root_entries`, `with_media`, `with_clock`)
  defaults everything from the device: FAT12 below 16 MiB, FAT16 below
  512 MiB, FAT32 above, with a cluster size adjusted until the count suits
  the variant, and a volume id derived from the clock, so `NoClock` gives
  reproducible images. `VolumeLabel::new` checks and uppercases a label.
  Errors: `NoSpace` for a device too small, `LimitExceeded` for one too
  large, `InvalidInput` for a bad option, `Unsupported` for blocks over
  4096 bytes. Every failure, including the final mount, is a
  `hadris_fs::MountError` that gives the device back. Checked with
  `fsck.fat` and `fsck_msdos`.
- **hadris-fat (V3):** `check(&mut fs) -> CheckReport` and
  `check_with(&mut fs, bitmap, on_finding)` in `sync`, `r#async` and
  `async_send` check a mounted `FatFs` read-only, without an allocator.
  They report each `Finding` (boot sector fields, the FAT32 backup boot
  sector and FSInfo sector, the free count, reserved FAT entries, FAT copy
  mismatches, invalid first clusters, broken, cyclic and cross-linked
  chains, bad clusters in chains, chains longer or shorter than their file,
  lost clusters, bad short names, dot entries, directory sizes, misplaced
  labels, long-name checksum mismatches and orphaned fragments), and
  `CheckReport` counts them by `FindingKind` with file, directory and
  cluster totals. The caller's bitmap sets the clusters tracked per pass;
  the tree is walked through `..` entries, so memory is fixed at any depth.
  Checked against `fsck.fat -n` verdicts and the crash-safety leftovers of
  interrupted operations. `FatFs::label` reads the root label entry.
- **hadris-tests:** The FAT conformance suite drives Hadris through
  `fat::generic::FsAdapter<M: Mount>`, one adapter over any `hadris-fs`
  `FileSystem`, with `HadrisFat` mounting `FatFs` on the image file. The
  V2 `FatVolume` adapter is removed.
- **hadris-fat (V3):** `FatFs::cluster_chain(node, visit)` passes each
  cluster of a node's chain to a callback without allocating, for tools that
  show file layout and fragmentation.
- **hadris-block, hadris (V3):** An additive `async-send` feature adds
  `hadris_block::async_send` (`OpenVolume` over `hadris_fat::async_send::FatFs`)
  and `detect::async_send`, generated from the same source as `r#async`, and
  enables `async-send` in `hadris-storage`, `hadris-fs`, `hadris-fat` and
  `hadris-part`. The umbrella `hadris` crate forwards
  `async-send` to `hadris-io`, `hadris-fs` and `hadris-block`, and its
  `sync` and `async` features now also reach `hadris-fs`.
- **hadris-macros (V3):** `send_async!`, a third generation mode next to
  `strip_async!`: every `async fn` in a trait declaration returns a `Send`
  future and the trait gains `Send` (and `Sync` for `&self` methods) as a
  supertrait.
- **hadris-io, hadris-storage (V3):** An `async-send` feature adds an
  `async_send` module generated from the same source as `r#async`, whose
  traits prove their futures `Send`, so generic code can be spawned on
  multi-threaded executors. `MaybeSend` marks the types that must be `Send`
  in that mode. `FromEmbedded` has no impls there.
- **hadris-fat (V3):** Depends on `hadris-storage`. The `sync`, `async`,
  `alloc` and `std` features also enable the same features of `hadris-fs`
  and `hadris-storage`, and a new additive `async-send` feature adds an
  `async_send` module, which holds the V3 `FatFs` driver.
- **hadris-storage (V3):** `BlockDevice`, one trait for sync and async
  whole-block devices with an explicit block size and the device's own error,
  implemented for `&mut D`, `Box<D>` and, with `std`, `std::fs::File`.
  `write_blocks` and `flush` return `WriteError<E>`, whose `ReadOnly` variant
  is how a device refuses writes; the default `write_blocks` returns it, so a
  read-only device implements no write method, and there is no `writable()`
  or access query. Devices and adapters: `StreamDevice` over any seekable
  stream (`ReadOnly` for streams without `Write`), `MemDevice` over byte
  buffers (error `OutOfRange`), `Slice` for a block range, `Cache` for
  write-back LRU caching (`alloc`, first write goes straight through), and
  `ByteView` for byte-granular access and a bounded stream. Adapters that can
  refuse a request report `StorageError<E>`.
- **hadris-fs (V3):** `DateTimeError`, `OpenOptionsError` and `PathError`
  convert into `Error<E>` with their kind, as `NameError` and `TableFull`
  already did, so `?` works from every `hadris-fs` validation error.
  `PathError::kind()` returns `ErrorKind::InvalidInput`.
- **hadris-block (V3):** `Error` reports the shared `ErrorKind` through
  `kind()` (`Io` for a device failure, the driver's kind for a failed
  mount, `Unsupported` for an unknown or unsupported format, `InvalidInput`
  for a partitioned disk, `Corrupt` when detection and the driver disagree)
  and the device error through `device_error()` and `into_device_error()`.
  `Error` and `OpenError` convert into `hadris_fs::Error`, keeping the kind
  and the device error, and with `std` into `std::io::Error`. `OpenError`
  gains `kind()` and `device()`.

### Changed

- **Fuzzing (V3):** `fs_dump` lists every filesystem through one generic
  walk over the `hadris-fs` `FileSystem` node API, with each driver wrapped
  in a `Volume`. Small seeds are committed for `cpio_read` (every format,
  a `.` entry, concatenated archives), `exfat_read` (a populated volume,
  which raises its coverage from 280 to over 1200 edges in a minute),
  `udf_read` (`mkudffs` volumes with 512-byte blocks) and `exfat_ops`.
- **Docs (V3):** The root README, crate READMEs, crate-level rustdoc,
  CONTRIBUTING, `tests/README.md` and the website describe the V3 API:
  exFAT is stable, NTFS is a preview, the `read`, `cache`, `lfn`,
  `unstable-streaming` and V2 type names are gone, and every Rust snippet
  compiles against the V3 crates. The website gains an exFAT formatting
  section, and CONTRIBUTING runs the FAT conformance filter as CI does
  (`fat:: -- --skip exfat::`). `hadris-fs`, `hadris-io` and
  `hadris-storage` build their docs.rs pages with every mode.
- **Docs site:** The documentation site is versioned. It serves the newest
  released minor at the root, every earlier released minor under `/X.Y/`
  and the unreleased docs under `/next/`, with a version dropdown and
  Docusaurus version banners. Snapshots are generated from the release tags
  at build time by `npm run versions`, and a published release rebuilds the
  site.
- **hadris-iso-cli, hadris-udf-cli, hadris-cpio-cli, hadris-cd-cli (V3):**
  The commands that do the same job share names and flags: `ls` has the
  alias `list` and `verify` the alias `check` in every CLI, and `extract
  --output` defaults to `.`. `extract --path` in the ISO and UDF CLIs
  writes a path other than the root to `<output>/<name>` as the FAT CLI
  does, through `extract_to_host` (the UDF CLI failed on a single file,
  and the ISO CLI merged the directory's contents into `--output`); ISO
  device nodes now stop the extraction with an error instead of a warning.
  `cat` streams through a `hadris-fs` `File` instead of reading the whole
  file first. `hadris-iso verify --strict` reads the path table through
  `raw::PathTableHeader`. `hadris-udf verify` walks the tree and reads
  every file, failing on errors, and `ls -a` adds `.` and `..` as in the
  ISO CLI. `hadris-cd verify` hashes file contents while it streams them
  instead of holding both trees in memory. `hadris-cpio extract` accepts
  the `.` entry that `find . | cpio -o` writes.
- **hadris-fat-cli (V3):** Supports exFAT. Every command detects exFAT
  from the boot sector through `exfat::raw::BootSector` and runs on
  `ExFatFs`: `info` shows the revision, sector and cluster size, FAT count
  and dirty flag, `ls`, `tree`, `cat`, `extract`, `stat`, `chain`,
  `fragmentation` and `verify` work as on FAT, and `create --fat-type
  exfat` formats with `exfat::sync::format`. `create` reads the source
  with `Tree::from_fs` before it formats. `verify` exits with an error when
  `check_with` reports findings (it exited 0 before), `extract --output`
  defaults to `.`, `ls` has the alias `list`, `verify` the alias `check`,
  and `--volume-label` the alias `--volume-name`.
- **hadris-fat (V3):** The `unstable-exfat` feature is gone: `exfat` is
  in every build and covered by semver. Its API is new; see Added and
  Removed.
- **hadris-io (V3):** `embedded-io` and `embedded-io-async` are optional,
  behind the `embedded-io` feature, which also gates `FromEmbedded` and
  the embedded conversions.
- **hadris-block (V3):** Opening exFAT no longer fails with
  `Detail::UnsupportedFormat`.

- **hadris-ntfs (V3):** The whole API is new; see Added and Removed.
  Features are `std`, `alloc`, `sync`, `async` and `async-send`, reading
  needs no allocator, and no feature changes what an item does. The boot
  sector is read from the device's first block instead of the stream's
  current position. Volumes with 4096-byte sectors now apply their update
  sequences correctly, and the root directory's `$INDEX_ROOT` in an
  extension record, which ntfs-3g writes, is followed. `hadris-common`,
  `bitflags` and `spin` are no longer dependencies.
- **hadris-block (V3):** `Error<E>` is a struct with `kind`, `detail`
  (`Detail::{UnknownFormat, PartitionedDisk, UnsupportedFormat,
  FormatMismatch, Mount}`) and `device_error`, and exists in every feature
  combination, as does `OpenError`. `OpenVolume` is a struct over a private
  driver enum instead of an enum with a `Fat` variant. Detection needs no
  feature, and `hadris-storage`, `hadris-fs`, `hadris-fat` and
  `hadris-ntfs` are always dependencies. The default features are `std`
  and `sync`.
- **hadris-optical (V3):** Detection and opening take a `hadris-storage`
  `BlockDevice` instead of a `hadris_io::legacy` stream, and detection
  reads the descriptors of devices with blocks of 512 to 4096 bytes.
  `Error<E>` is a struct with `kind`, `detail`
  (`Detail::{UnknownFormat, FormatUnavailable, Mount}`) and
  `device_error`, and exists in every feature combination. `OpenOpticalImage`
  is a struct over a private driver enum. `hadris-iso`, `hadris-udf`,
  `hadris-storage` and `hadris-fs` are always dependencies, and the default
  features are `std` and `sync`.
- **hadris (V3):** No longer reaches formats through the facades:
  `hadris::fat`, `hadris::iso` and the other formats are the format crates
  themselves. `block` adds `hadris-block` with `fat` and `part`, `optical`
  adds `hadris-optical` with `iso`, `udf` and `cd`, and `write` forwards
  FAT formatting. The default features are `std`, `sync`, `write`, `fat`,
  `iso` and `cpio`.
- **hadris-udf (V3):** The whole API is new; see Added and Removed.
  Features are `std`, `alloc`, `sync`, `async` and `async-send`, and no
  feature changes what an item does. Volumes are byte for byte those of
  2.4 with the same clock, except for fixes: UDF 2.00 and later write
  descriptor version 3 and the file's unique id in identifier ICBs, the
  integrity descriptor counts files and directories, a directory's link
  count includes its subdirectories, the root is its own parent, and an
  identifier's tag location is the block that holds it. The volume is
  written from block 0 to its end, so a growing host file gets its full
  length. A volume identifier over 126 bytes fails instead of being cut;
  the 32-byte fields still take a prefix, now cut at a character.
  `hadris-common` is no longer a dependency.
- **hadris-cd (V3):** The whole API is new; see Added and Removed. Images
  are those of 2.4 with the same clock except for the UDF fixes above and
  a correct next unique id in the integrity descriptor; the image length
  is at least the ISO image plus the trailing anchor. Features are `std`,
  `sync`, `async` and `async-send`; the crate needs an allocator but not
  `std`.
- **hadris-cpio (V3):** Features are `std`, `alloc`, `sync`, `async` and
  `async-send`; no feature changes what an item does. Archives without
  hard links are byte for byte those of 2.4. Hard link groups carry the
  data on the last name, and every name has the group's link count and
  metadata, as GNU cpio writes them. `hadris-common` is no longer a
  dependency.
- **hadris-optical (V3):** Opens UDF through `hadris_udf` `UdfFs` over the
  transitional `StreamBlocks` adapter; `Error::Udf` holds a
  `hadris_udf::Error`. `async-send` and the mode features reach
  `hadris-udf` and `hadris-cd`, and `read` and `write` no longer forward to
  `hadris-udf`.
- **hadris-udf-cli (V3):** Reads through `UdfFs` and the `hadris-fs` path
  helpers: `ls -l` shows symlinks, `cat` takes any path, `extract` uses
  `extract_to_host` (so hostile names cannot escape the target), and
  `info` lists the partitions. `create` builds a `Tree` from the source
  directory with its symlinks, hard links and metadata, stamps it with the
  current time and prints the writer's warnings.
- **hadris-cd-cli (V3):** `create` stores symbolic links (Rock Ridge and
  UDF) instead of refusing them, writes to the output file directly, and
  prints the writers' warnings; `info` and `verify` read UDF through
  `UdfFs`.
- **hadris-cpio-cli (V3):** `-` reads the archive from standard input and
  `create -o -` writes to standard output. `create` has
  `--format newc|crc|odc` (`--crc` still works) and `--verbose`, and stores
  device nodes and hard links from the source directory. `extract`
  restores hard link groups and refuses names with `..` components or that
  lead through an existing symlink.
- **hadris-iso (V3):** The whole API is new; see Added and Removed.
  Features are `std`, `alloc`, `sync`, `async` and `async-send`, and no
  feature changes what an item does. Images written without Rock Ridge
  are byte for byte those of 2.4 with the same clock; with Rock Ridge, PX
  serial numbers are per node and shared by hard links, and `..` carries
  the parent's attributes. Joliet and enhanced trees keep the real
  hierarchy instead of the relocated one. Without Rock Ridge, symlinks and
  device nodes are left out with a warning instead of failing the write.
  The relocation directory is named by `RockRidge::with_relocation`
  (`rr_moved` by default) and a clash at the root fails instead of
  falling back to `.rr_moved`. Files of 4 GiB or more need `IsoLevel::L3`.
  `hadris-common` is no longer a dependency.
- **hadris-optical (V3):** Opens ISO images through `hadris_iso` `IsoImage`
  over a transitional `StreamBlocks` adapter for its `hadris_io::legacy`
  streams; `Error::Iso` holds a `hadris_iso::Error`. A new `async-send`
  feature enables it in `hadris-iso`, and `read` and `write` no longer
  forward to `hadris-iso`. The umbrella `hadris` crate forwards
  `async-send` to it.
- **hadris-cd (V3):** Writes its ISO part from a `hadris_fs::tree::Tree`
  with `hadris_iso::sync::write` and takes file positions from the
  `Report` instead of reading the image back. `IsoOptions` has `level`
  and `name_case` fields (`IsoLevel`, `NameCase`), `joliet` takes a
  `JolietLevel` (`L1` to `L3`), `rock_ridge` a `RockRidge`, `boot` an
  `ElTorito` and `hybrid_boot` a `HybridBoot`. UDF always keeps the
  original names; before, an image without Joliet gave UDF the ISO names.
- **hadris-iso-cli (V3):** Reads through the preferred tree with the
  `hadris-fs` path helpers: listings show Rock Ridge or Joliet names, a
  path not found there is looked up in the primary tree ignoring ASCII
  case, `extract` writes symlinks, and `info` lists the boot catalog.
  `verify` checks the boot catalog entries, and with `--strict` the path
  table, extent bounds and Rock Ridge fields through `raw`. `create` and
  `mkisofs` build a `Tree` from the source directory, stamp entries with
  the current time and print the writer's warnings (dropped metadata only
  with `--verbose`); `create --dry-run` prints the planned size, and
  `mkisofs --isohybrid-mbr` uses the file as MBR boot code.
- **hadris-cd-cli (V3):** Uses the V3 boot and hybrid options, keeps the
  boot catalog hidden so both namespaces list the same files, and
  `verify` compares contents only when the ISO tree has just ISO 9660
  names.
- **Fuzzing and tests (V3):** `iso_read` walks every tree an image carries
  through the `FsDriver` methods and reads descriptors, the boot catalog,
  Rock Ridge entries, raw records, extents and link targets; `fs_dump`
  lists the preferred tree through the shared driver dump. The ISO
  conformance suite writes `Tree`s and reads through views.
- **hadris-io (V3):** The crate root no longer glob re-exports the `sync`
  module (R5). Name the traits through their mode: `hadris_io::sync::Read`,
  `hadris_io::r#async::Read` or `hadris_io::async_send::Read`. The root keeps
  the mode-independent items (`ErrorType`, `ExactError`, `Cursor`,
  `SeekFrom`, `FromEmbedded`, `StdIo`, `ToStd`).
- **hadris-storage (V3):** `BlockIndex` and `BlockCount` have a private
  field: build them with `BlockIndex::new(n)` and read them with `get()`
  (R2). `BlockRange` (`start`, `count`) and `BlockGeometry`
  (`logical_block_size`, `block_count`, `physical_block_size`) have
  `const fn` accessors instead of public fields. `StreamWrite` is sealed.
- **hadris-fat (V3):** The variants of `Finding` with fields are
  `#[non_exhaustive]`, so findings are made only by `check` and a later
  release can add a location to one; match them with `..`.
- **hadris-part (V3):** CRCs are always computed and checked; the `crc`
  feature is gone. No GUID is made at random: `Gpt::new(disk_guid, ..)` and
  `GptEntry::new(type_guid, unique_guid, ..)` take GUIDs, and
  `Guid::random` (with `std`, no `rand` dependency) makes one on request.
  `Guid` implements `FromStr` (with or without braces); the inherent
  `from_str` is now `Guid::parse_const`, and `Guid::UNUSED` is `Guid::NIL`.
  The type GUID constants move from `Guid` to `gpt::types`
  (`gpt::types::EFI_SYSTEM`). `MbrType(u8)` with associated constants
  (`MbrType::FAT32_LBA`, `MbrType::LINUX`, `MbrType::EXTENDED_LBA`, ...)
  replaces `MbrPartitionType` and `MbrPartitionTypeFull`. `0x04` is
  `FAT16_SMALL`, `0x06` is `FAT16` and `0x0E` is `FAT16_LBA`; the old enum
  called `0x04` `Fat16` and `0x06` `Fat16Lba`.
  `Gpt` has private fields. `PartitionFlags` replaces the `bootable` bool
  parameters. Features are `std`, `alloc`, `sync`, `async` and
  `async-send`; `read`, `write`, `crc` and `rand` are removed, reading and
  writing are always available, and the `endian-num` and `rand`
  dependencies are dropped. Block sizes must be powers of two of at least
  512 bytes. `write` refuses a GPT whose copies would not fit beside the
  usable area, as on a truncated image, instead of overwriting partition
  data. The `part_read` fuzz target also scans, edits, writes and re-reads
  what it parses, and `fuzz/scripts/gen-seeds.sh` makes small MBR, EBR,
  GPT (512 and 4096-byte blocks), hybrid and damaged-GPT seeds that fit
  the target's 64 KiB inputs.
- **hadris-block (V3):** The `read` and `write` features no longer enable
  anything in `hadris-part`.
- **hadris-fs (V3):** The driver contract is written down in full on
  `FsDriver`: `NodeId` 0 is never a node, cursors stay at or below the new
  `DirCursor::MAX_RAW` (`2^63 - 16`), reads never change times, pending
  fields may lag until `publish_node`, a dropped async call leaves no pin,
  and a listed id is the id `lookup` returns for that name. The trait
  documents how it grows after 3.0 (defaults, `also = [..]` in
  `impl_fs_driver!`, wrappers forward new methods themselves), and a test
  fails when a wrapper (`&mut F`, `Box`, `&F`, `AsDriver`, `Volume`,
  `WithResolver`, `Arc`, `Rc`, the macro) misses a trait method.
- **hadris-fat (V3):** `MountOptions::with_read_only()` takes no argument
  (R9: no bool parameters); mounts are writable unless it is called.
- **hadris-fat (V3):** A node id is the slot of its directory entry plus a
  tier in the bits above bit 40, which counts up only while a pinned node
  that has moved away holds the slot's lower tiers. `read_dir_entry` and
  `lookup` therefore report the same id for an entry, where before a
  listing could report a "moved" id and a lookup a "fallback" id for the
  same file after a rename. Ids are never 0 and stay below `2^63`.
- **hadris-storage (V3):** `BlockDevice::flush` for `std::fs::File` calls
  `sync_data`, so `sync` and `sync_node` on a host image or block device
  reach stable storage. It used to call `Write::flush`, which does nothing
  for a file.
- **hadris-fs (V3):** `publish_node` writes a node's pending metadata
  without flushing the device (default: `sync_node`), and `sync_node` is
  documented as durable, like `fsync`. `File::close`, `OpenFile::close` and
  `File`'s `Write::flush` publish; the new `File::sync_all` and
  `OpenFile::sync_all` call `sync_node`. `copy_tree` and `import_from_host`
  publish each file and leave the device flush to `sync`. `FatFs`
  implements `publish_node`, so closing a written file no longer flushes
  the device.
- **hadris-fs (V3):** Pins and opens are separate. `lookup`, `create` and
  `parent` pin a node, which never blocks removal; the new `open_node` and
  `close_node` (defaults do nothing) mark a pinned node as open. `remove` and
  a replacing `rename` fail with `ErrorKind::Busy` only for the last name of
  an open node. A pinned node that is removed keeps its id until its last
  `forget`, and every other method answers `ErrorKind::NotFound` for it.
  `File` and `OpenFile` open their node and close it on `close` or drop;
  `File::from_pinned` and `OpenFile::from_pinned` are now `async`, take the
  driver and return a `Result`. `Volume::close_node` never blocks: like
  `forget`, it queues the call when the lock is held. `impl_fs_driver!`
  forwards `open_node` and `close_node` when named in `also = [..]`.
  `FatFs` implements both. A FUSE mount, which keeps a lookup on every
  cached name, can now remove and replace files it has looked up.
- **hadris-fs (V3):** `remove` takes a `RemoveKind` (`File`, `Dir`,
  `Any`; non-exhaustive): `remove(dir, name, kind)`. The driver checks the
  type it already reads, failing with `IsADirectory` or `NotADirectory`, so
  `unlink` and `rmdir` need no lookup first. `RemoveKind::check` does the
  comparison for drivers. `remove_file`, `remove_dir` and `remove_dir_all`
  no longer pin the node they remove.
- **hadris-fs (V3):** `ErrorKind` gains `NameTooLong` and `FileTooLarge`,
  so each kind maps to one errno. A name longer than the format or a
  `NameBuf` accepts (`NameError::TooLong`) and an over-long FAT name or
  volume label give `NameTooLong`; a FAT write or `set_len` past 4 GiB - 1
  gives `FileTooLarge`. `LimitExceeded` keeps full node tables, long paths
  and values that do not fit a field or buffer. With `std` they convert to
  `io::ErrorKind::InvalidFilename` and `FileTooLarge`. `Error<E>` equality
  compares the kind and the device error only, now and after context is
  added.
- **hadris-fat (V3):** The `write` feature only adds `format` in each mode
  and no longer implies `read` or `alloc`; `FatFs` reads and writes without
  it. Default features are `std`, `sync` and `write`. The crate root no longer
  re-exports the `sync` module: write `hadris_fat::sync::FatFs`. The
  `unstable-exfat` preview keeps its API but has its own
  `hadris_fat::exfat::Error` and `Result`, reads through
  `hadris_io::legacy::sync` directly, and with `std` stamps new entries with
  the UTC time of `hadris_fs::SystemClock` instead of chrono's local time.
- **hadris-block (V3):** `detect::sync::detect` and `detect::r#async::detect`
  take a `hadris-storage` `BlockDevice` and return the device's error; the
  block size is the device's. `OpenVolume<D>` takes the device by value and
  holds a `FatFs<D>`, with `into_inner` returning the device. `Error<E>`
  carries the device error in `Device` and the mount error as
  `Fat(hadris_fs::Error<E>)`. `OpenVolume::open` and `open_detected` fail
  with `OpenError<D, E>`, which carries the `Error` and always gives the
  device back (`error`, `into_error`, `into_device`, `into_parts`) instead
  of dropping it. The volume is mounted once, and a failed mount returns
  the device through `FatFs`'s `MountError`. `?` converts an `OpenError`
  into `Error`. A partition becomes a `Slice<D>` of the disk through
  `part::sync::open` (and its async forms) from `hadris-part`. The `detect` feature
  depends on `hadris-storage` instead of `hadris-io`. The `sync` and
  `r#async` openers are generated from one source.
- **hadris-fat-cli (V3):** Every command runs on `FatFs`; read commands mount
  images read-only. `stat` counts clusters, files and directories with
  `check` and no longer prints reserved clusters. `verify` runs `check_with`,
  prints each finding and "Clusters In Use" instead of "Clusters Verified",
  and with `--verbose` adds free, bad and lost clusters. `fragmentation` and
  `chain` read chains with `FatFs::cluster_chain`. `create` rejects volume
  labels longer than 11 ASCII characters, formats with `format`, imports with
  `import_from_host` in name order and stamps entries with the current UTC
  time; `extract` uses `extract_to_host` and restores modification times.
  `extract --path` names its output after the entry's stored name rather
  than the typed path, so `-p /sub/readme.txt` writes `README.TXT` as the
  image spells it, and a path that resolves to the root, such as `/Sub/..`,
  extracts the whole image into `--output` instead of writing beside it.
- **Fuzzing and examples (V3):** `fat_read`, `fat_ops` and `fs_dump` drive
  `FatFs`; `fat_ops` also covers `write_at` and `set_len` and asserts that
  `check` finds nothing after `sync`. The `fat-list` and `shared_volume`
  examples use `FatFs` and the `hadris-fs` path helpers and `Volume`.
- **hadris-io (V3):** `Read`, `Write` and `Seek` (sync and async) report the
  implementor's own error through the new `ErrorType` supertrait, as in
  `embedded-io`. The error only needs `core::error::Error + Send + Sync +
  'static`, so a kernel uses its own enum and a device error reaches the
  caller unchanged without allocation. `read_exact` and `write_all` return
  `ExactError<E>`. `&mut T` and `Box<T>` implement the traits, so the
  `Borrowed` wrapper is gone. The blanket impls over `embedded-io` and
  `std::io` types are replaced by explicit adapters: `FromEmbedded<T>` (error
  `T::Error`), `StdIo<T>` (error `std::io::Error`) and `ToStd<T>`. With `std`,
  `std::fs::File` has `std::io::Error` as its error, `into_std_error` converts
  any device error to `std::io::Error` (returning an `io::Error` as itself),
  and `ExactError<E>` converts with `?`. `Cursor` reports `InvalidSeek`.
  `ByteSource` and `SeekSource` add a positional byte source for writers.
  The V2 traits with the erased `Error`, `ErrorKind`, `Result`, `ReadExt`,
  `Parsable` and `Writable` move to `hadris_io::legacy`, which format crates
  use until they are ported and which is removed before 3.0. `Error::erase`,
  `Error::from_source`, `IoError` and `ToEmbedded` are removed.
- **All format crates (V3):** Use `hadris_io::legacy` for now. Error types
  wrap the non-generic `hadris_io::legacy::Error`, and generic bounds no
  longer spell out `Seek<Error = ...>`.
- **hadris-storage (V3):** `PartitionView` implements the `hadris_io::legacy`
  traits and `PartitionView::new` returns `hadris_io::legacy::Result`. The old
  `BlockDevice`, `BlockDeviceMut`, `SeekBlockDevice` and the crate's own error
  type are removed.
- **hadris (V3):** Re-exports `hadris-io` as `hadris::io`.

### Removed

- **hadris-fat (V3):** The exFAT preview API: `ExFatVolume`, `ExFatInfo`,
  `ExFatDir`, `ExFatDirIter`, `ExFatFileEntry`, `ExFatFileReader`,
  `ExFatFileWriter`, `ExFatFormatOptions`, `ExFatLayoutParams`,
  `calculate_layout`, `format_exfat`, `exfat::Error` and `Result`, and the
  public allocation and hashing helpers. Use `ExFatFs`, `exfat::raw` and
  `exfat::FormatOptions`.
- **hadris-io (V3):** `hadris_io::legacy`, the last V2 stream traits.

- **hadris-ntfs (V3):** The V2 API: `NtfsFs` over `hadris_io::legacy`
  streams with `root_dir`, `open_path` and `read_mft_record`, `NtfsDir`,
  `NtfsEntry`, `FileReader`, `NtfsFsReadExt`, `NtfsError` and `Result`,
  the public `attr` module (`AttrIter`, `NtfsAttr`, `AttrBody`, `DataRun`,
  `DataRunDecoder`, `FileNameInfo`, `IndexEntryInfo`, `parse_file_name`,
  `parse_index_entries`, `apply_fixups`, `decode_data_runs`,
  `decode_record_size`, `decode_utf16le`), `RawNtfsBootSector` (now
  `raw::BootSector`), the glob re-exports of `sync` and `raw` at the root,
  and the `read` feature. Use `NtfsFs` in a mode module with the node API
  or the `hadris-fs` path helpers.
- **hadris-block (V3):** The `Error` enum variants (`Device`,
  `UnknownFormat`, `PartitionedDisk`, `UnsupportedFormat`,
  `DetectedFormatMismatch`, `Fat`), the `Result` alias,
  `OpenVolume::Fat`, and the `read`, `detect`, `storage` and `fat`
  features.
- **hadris-optical (V3):** The transitional `StreamBlocks` adapter,
  `OpenOpticalImage<'a, S>` over borrowed legacy streams with
  `as_iso9660` and `as_iso9660_mut`, the `Error` enum (`Io`,
  `UnknownFormat`, `RequestedFormatUnavailable`, `Iso`, `Udf`) and the
  `Result` alias, and the `read`, `write`, `detect`, `open`, `iso` and
  `udf` features. `hadris-optical` no longer uses `hadris_io::legacy`.
- **hadris (V3):** The `read`, `fs` and `storage` features; the formats'
  `read` features they forwarded are gone.
- **hadris-udf (V3):** The V2 API: `UdfVolume` over `hadris_io` streams
  with `UdfVolumeInfo`, `UdfDir`, `UdfDirEntry`, `read_file`,
  `read_directory` and `root_dir`, `UdfWriter` and its public descriptor
  writers (`write_vrs`, `write_avdp`, `write_pvd`, `write_lvid(close:
  bool)`, `write_fids` and the rest), `UdfWriteOptions`,
  `UdfCreateOutput`, `SimpleFile`, `SimpleDir`, `FileSource`,
  `UdfFileInfo`, `UdfDirInfo`, `UdfFileExtent`, the `descriptor`, `dir`
  and `file` modules with their layouts (now in `raw`), the uncompiled
  `modify.rs`, the V2 `Error` and `Result`, the `read`, `write` and
  `unstable-streaming` features and the root re-export of `sync`. Use
  `UdfFs`, `write` and `plan` in a mode module, `UdfOptions`,
  `hadris_fs::tree::Tree`, and the layouts in `raw`.
- **hadris-cd (V3):** `OpticalImageWriter` (`new`, `finish`, `create`),
  `OpticalImageOptions` with its `iso_only` and `udf_only` switches and
  `sector_size`, the crate's own `IsoOptions` and `UdfOptions` structs,
  `FileTree`, `Directory`, `FileEntry`, `FileData`, `FileExtent`,
  `LayoutManager`, `LayoutInfo`, the V2 `Error` and `Result`. Use
  `write` and `plan` in a mode module with `CdOptions`; an ISO-only or
  UDF-only image is `hadris_iso` or `hadris_udf` `write`.
- **hadris-cpio (V3):** The V2 API: `CpioArchiveReader`, `CpioEntry`,
  `CpioEntryOwned`, `CpioEntryHeader`, `CpioArchiveWriter`,
  `CpioWriteOptions`, `FileTree`, `FileNode`, `FromFsError`,
  `mode::FileType` and `make_mode`, `RawNewcHeader` and its 14-argument
  `build`, `CpioMagic`, the V2 `Error` and `Result`, the `read` and
  `write` features, and the root re-export of `sync`. Use `CpioReader`,
  `CpioWriter` and `write` in a mode module, `CpioOptions`,
  `hadris_fs::tree::Tree`, and the layouts in `raw`.
- **hadris-common (V3):** The `extent` module (`Extent`, `FileType`) and
  the `layout` module (`FileLayout`, `DirectoryLayout`, `AllocationMap`),
  which only the V2 optical writers used, and the `alloc`, `std`, `sync`
  and `async` features with the `hadris-io` and `hadris-fs` dependencies.
  The crate now holds only the endian integers.
- **hadris-iso (V3):** The V2 API: `IsoImage` over `hadris_io` streams
  with `IsoDir`, `DirEntry`, `DirectoryRef`, `RootDir`, `IsoFileReader`,
  the allocation-free `IsoReader`, `IsoRoot`, `IsoDirEntry` and
  `IsoCursor`, `IsoImageWriter`, `IsoFormatOptions`, `CreationFeatures`,
  `BaseIsoLevel`, `InputFiles`, `InputTree`, `InputEntry`,
  `InputMetadata`, `FileSource`, the `estimator` (`IsoSizeEstimate`,
  `SizeBreakdown`), `IsoModifier` and `ModifyOp`, `BootOptions`,
  `BootEntryOptions`, `BootSectionOptions`, `EmulationType`,
  `PlatformId`, `HybridBootOptions`, `RripOptions`, `RripBuilder`,
  `RripMetadata`, `SystemUseIter` and `SystemUseField`, `PathSeparator`,
  `LogicalSector`, the `IsoStr*` and `IsoString*` character-set types,
  `FilenameL1`, `FilenameL2`, `FilenameL3`, `EntryType`, the V2 `Error`
  and `Result`, the `read`, `write`, `joliet` and `unstable-streaming`
  features and the `hadris_iso::io` re-export. Use `IsoImage`, `IsoView`,
  `write`, `plan` and `Session` in a mode module, `IsoOptions` and its
  option types, `hadris_fs::tree::Tree`, and the layouts in `raw`.
- **hadris-common (V3):** `EndianType`, the fixed-capacity types
  (`FixedBytes`, `FixedStr`, `FixedUtf16`, `ArrayVec`, `RingBuf`),
  `BOOT_SECTOR_BIN` and the boot sector project that built it, and the
  `heapless` dependency, which only the V2 ISO crate used.
  `Endianness::get` is now `is_le`.
- **hadris-storage (V3):** `PartitionView`, which bounded a
  `hadris_io::legacy` stream. `Slice` replaces it over a `BlockDevice`.
  `hadris-storage` no longer depends on `embedded-io`.
- **hadris-common (V3):** `MaybePod`, whose bounds changed with the
  `bytemuck` feature; the `Endian` associated types have no bounds now and
  `bytemuck` only adds `Pod` impls. The unused `optical` module and
  feature, the `alg` module (`Crc32HasherIsoHdlc`) and the `chrono`, `crc`
  and `rand` dependencies, which `std` pulled into every dependent crate.
- **hadris-part (V3):** The V2 API: `MasterBootRecord`, `MbrPartition`,
  `MbrPartitionTable`, `MbrPartitionType`, `MbrPartitionTypeFull`,
  `GptHeader`, `GptPartitionEntry`, `GptPartitionName`, `GptAttributes`,
  `GptDisk`, the V2 `PartitionTable`, `PartitionInfo`, `PartitionType`,
  `PartitionSchemeType`, `detect_scheme_from_mbr`, `is_hybrid_mbr`,
  `HybridMbrBuilder`, `HybridMbrConfig`, `MirroredPartition`,
  `DiskGeometry` and the alignment helpers, the six `*ReadExt` and
  `*WriteExt` traits, `PartitionInfoTrait`, `PartitionTableRead`,
  `sync::partition_table::{detect, open}`, the crate-level `Result` and the
  root re-exports of the `sync` module. Use `Disk` with `read`, `write`,
  `create`, `scan` and `open` in a mode module, `Mbr`, `Gpt`, `Hybrid`,
  `HybridMbr`, `DiskLayout`, `Partition` and the layouts in `raw`.
- **hadris-fat (V3):** The V2 FAT12/16/32 API: `FatVolume`,
  `FatVolumeBuilder`, `FatDir`, `FileEntry`, `DirectoryEntry`, `FileReader`,
  `FileWriter`, `FatVolumeReadExt`, `FatVolumeWriteExt`, the `fat_table`
  types (`Fat`, `Fat12`, `Fat16`, `Fat32`, `FatType`), the FAT sector cache
  (`FatSectorCache`, `CachedFat`), the V2 `format` module
  (`FatVolumeFormatter`, `FatFormatOptions`, `FatTypeSelection`,
  `SectorSize` and the layout calculator), the `tool` analysis and verify
  extensions (`FatAnalysisExt`, `FatVerifyExt` and their reports), `time`
  (`FatDateTime`, `TimeProvider`), `oem` (`OemCpConverter`), `file`
  (`ShortFileName`, `LongFileName`, `LfnBuilder`), the crate-level `Error`
  and `Result`, `hadris_fat::io`, and the raw directory entry types
  (`RawFileEntry`, `RawLfnEntry`, `RawDirectoryEntry`, `DirEntryAttrFlags`,
  `NtCaseFlags`). Use `FatFs`, `format`, `check` and `check_with`, and
  `hadris_storage::sync::Cache` for caching. The `read`, `lfn`, `cache`,
  `tool` and `dirty-file-panic` features, the `chrono` dependency and the
  `HADRIS_FAT_CACHE_WINDOW_SIZE` build variable are removed. Library-level
  fragmentation analysis is not ported; the CLI computes it from
  `cluster_chain`.
- **hadris-block (V3):** Detection and opening over `hadris_io::legacy`
  streams, `mbr_partition_view`, `gpt_partition_view` and `Error::Io`.

- **hadris-path (V3):** Merged into `hadris-fs` as `hadris_fs::path`.
  `Component`, `Separators` and `PathError` are now `#[non_exhaustive]`. The
  umbrella `hadris` crate replaces its `path` feature and `hadris::path` module
  with `fs` and `hadris::fs`.

- **hadris-fixed (V3):** Folded into `hadris-common` as `types::fixed`, which
  also absorbs the former `types::no_alloc` (`ArrayVec`, `RingBuf`).
  `FixedUtf16` now takes the `hadris_common::types::endian` byte-order markers,
  so `Utf16ByteOrder` and the duplicate `LittleEndian`/`BigEndian` markers are
  gone. `ArrayVec::try_push` returns `CapacityError` instead of the removed
  `ArrayVecError`. The umbrella `hadris` crate drops its `fixed` feature and
  `hadris::fixed` module. `hadris-common` is documented as internal and not for
  direct use.
- **hadris-archive (V3):** Removed. The umbrella `hadris` crate depends on
  `hadris-cpio` directly and re-exports it as `hadris::cpio` instead of
  `hadris::archive::cpio`. The `archive` and `cpio` features are unchanged.

### Fixed

- **hadris-fs (V3):** Dropping a written `File` without `close` in the
  blocking API publishes its size and times, ignoring errors, so a file
  dropped before a power cut keeps its data reachable. On a `Volume` whose
  lock is held, the publish waits in the queue with the close. The async
  APIs cannot await in `Drop`; there a dropped file's metadata stays
  pending in the driver until the next `publish_node`, `sync_node` or
  `sync`, as the `File` docs say.
- **hadris-fs (V3):** `Volume` no longer deadlocks (`StdMutex`, `Spin`) or
  spins forever (`Local`) when handles to more than 16 distinct nodes are
  dropped while `lock()` is held. With `alloc` the deferred queue grows;
  without it, the 17th node panics with a message naming the cause. A call
  on a `Local` volume while its `lock()` guard is held panics with a
  message naming the cause instead of a bare `RefCell` borrow error.
- **hadris-fs, hadris-fat (V3):** `hadris-fat` builds for targets without
  atomic compare-and-swap, such as `thumbv6m-none-eabi` and
  `riscv32imc-unknown-none-elf`. It no longer depends on `spin` or
  `bitflags`, which it did not use. `hadris-fs` uses the volume's own lock
  for its queue, so `spin` is needed only by the `Spin` lock, which exists
  on targets with `target_has_atomic = "ptr"`.
- **hadris-iso (V3):** Joliet and enhanced volume descriptors pad their
  escape sequence field with zeros, as ECMA-119 8.5.6 requires, instead of
  spaces. libarchive (`bsdtar`) refused every image with an enhanced tree,
  including every `hadris-cd` image, and listed it as an empty archive.
- **hadris-macros:** `send_async!` no longer panics on a trait without a
  body or an `async fn` with neither a body nor a semicolon, and no longer
  takes the next item's body for a trait alias. It passes such tokens
  through, so rustc reports the error at the user's span.
- **hadris-fat:** Generated short names no longer turn a non-ASCII
  character whose code point ends in an ASCII byte into that byte (U+0121
  became `!`, U+012E was dropped as a `.`), ignore code page bytes below
  `0x80`, and uppercase non-ASCII characters before the code page maps them,
  so the `Cp437` code page stores `é` as `É` (`0x90`).
- **hadris-iso:** Write Rock Ridge relocation placeholders compatible with
  libarchive/bsdtar and use only recognized relocation container names. Reject
  relocation when a root `rr_moved` directory would be mistaken for the container
  or both supported names are occupied, instead of producing an unreadable image.
- **hadris-iso-cli, hadris-udf-cli, hadris-cd-cli, hadris-fat-cli,
  hadris-cpio-cli:** `create` writes to a temporary file in the output's
  directory and renames it into place once the image is complete. A failed
  `create` no longer truncates an existing output (`hadris-iso create -V`
  with a name that is too long emptied it) or leaves a partial image
  behind. An existing device or other non-regular file is still written in
  place, and `hadris-fat create` still refuses an existing output.
- **hadris-cd-cli:** `verify` accepts a Rock Ridge image whose deep
  directories were relocated: the relocation directory, which only the ISO
  side holds, is no longer reported as a mismatch.

## [2.4.0] - 2026-09-08

### Added

- **hadris-iso (`unstable-streaming`):** `InputEntryKind::Source` streams a
  file's contents from a reader that is opened while the image is written, so a
  large input no longer has to be held in memory. `FileSource::from_path`
  streams a file on disk. The feature is outside the V2 stability promise.
  ([@zone117x](https://github.com/zone117x),
  [#111](https://github.com/hxyulin/hadris/pull/111))
- **hadris-udf (`unstable-streaming`):** `SimpleFile::from_source` streams a
  file's contents from a `FileSource` that is opened while the image is
  written. The feature is outside the V2 stability promise.
  ([@zone117x](https://github.com/zone117x),
  [#115](https://github.com/hxyulin/hadris/pull/115))

### Fixed

- **hadris-fat (`unstable-exfat`):** The exFAT writer now records a file's size
  as the stream extension's `DataLength` instead of its cluster-rounded
  allocation, and an empty file keeps `AllocationPossible` set with
  `NoFatChain` clear so both `fsck_exfat` and `fsck.exfat` accept it.
  ([@zone117x](https://github.com/zone117x),
  [#116](https://github.com/hxyulin/hadris/pull/116))
- **hadris-iso:** Directory iteration now stops after a malformed record or I/O
  error instead of returning the same error on every call.
  ([@zone117x](https://github.com/zone117x),
  [#113](https://github.com/hxyulin/hadris/pull/113))
- **hadris-iso:** Images too large for an MBR partition entry or for the
  32-bit ISO 9660 volume space size now fail with `InvalidInput` instead of
  writing a truncated sector count.
  ([#105](https://github.com/hxyulin/hadris/issues/105))
- **hadris-iso:** Path table iteration now stops after an I/O or parse error
  instead of returning the same error on every call.
- **hadris-udf:** Files of 1 GiB or more are now recorded with several short
  allocation descriptors. The single descriptor written before overflowed its
  30-bit extent length, so such files read back empty or truncated.
- **hadris-udf (`unstable-streaming`):** An interrupted read from a
  `FileSource` is retried instead of failing the image.

## [2.3.0] - 2026-09-02

### Added

- **hadris-iso:** The ISO writer now emits multi-extent files larger than 4 GiB.
  The allocating reader can consume them incrementally with
  `read_file_chunked`.
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#96](https://github.com/hxyulin/hadris/pull/96))
- **hadris-part:** Added `MbrPartition::new_iso_partition` for whole-image
  ISO 9660 partitions in isohybrid-style MBRs.
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#96](https://github.com/hxyulin/hadris/pull/96))

### Changed

- **Tests:** FAT and ISO now use the same hosted, external-tool, and manual
  test tiers. CI requires mtools and dosfstools for FAT interoperability and
  xorriso for ISO interoperability, while local runs skip unavailable tools.
- **Development:** The pinned Rust toolchain now installs `rust-analyzer` for
  editor support. ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#100](https://github.com/hxyulin/hadris/pull/100))

### Fixed

- **hadris-iso:** Multi-extent directory planning now counts continuation
  records before assigning file data, preventing creation failures when a
  continuation crosses a directory-sector boundary. `WrittenFile` retains its
  v2.2 field layout for source compatibility.
- **hadris-fat:** FAT name handling is now case-insensitive for long names,
  rejects reserved characters, permits case-only renames, prevents moving a
  directory into its own subtree, and generates valid aliases for leading-dot
  names. Readers still accept existing nonconforming names and prefer an exact
  long-name match when an old image contains entries that differ only by case.
- **hadris-fat:** Failed writes caused by a full data region now release their
  uncommitted clusters and persist a valid FAT32 FSInfo count and next-free
  hint instead of leaving orphaned allocations or stale metadata.
- **hadris-fat:** File and directory creation and rename now allocate distinct
  8.3 aliases when long filenames share the same generated short name. Such
  entries previously appeared through their long names but were rejected as
  duplicate directory entries by `fsck.fat`.
- **hadris-iso:** Non-final extents of multi-extent files are now sized as a
  multiple of the configured logical block size instead of always 2048 bytes,
  as required by ECMA-119 6.5.4.

## [2.2.0] - 2026-08-24

### Added

- **hadris-fat:** `FileReader` now supports start-, current-, and end-relative
  seeking in both the synchronous and asynchronous APIs, including buffered
  and cached-chain readers.
  ([@stlankes](https://github.com/stlankes),
  [#89](https://github.com/hxyulin/hadris/pull/89))
- **hadris-fat:** New `Error::StaleEntry` (with the `write` feature) reported by
  the mutating APIs and `FileReader` when a `FileEntry` handle no longer refers
  to the same on-disk directory entry (see below).
- **hadris-fat:** New `Error::WriterConflict` (with the `write` feature) returned
  when a second `FileWriter` is opened for a directory entry that already has one
  open (see below).
- **hadris-iso:** New `HybridBootOptions::efi_boot_partition` field and
  `with_efi_boot_partition` builder to expose an El Torito UEFI boot image as a
  GPT EFI System Partition (the equivalent of xorriso's
  `-efi-boot-part --efi-boot-image`). When unset, the writer automatically uses
  the boot image of the single `PlatformId::UEFI` El Torito entry, if there is
  exactly one, so existing UEFI/hybrid consumers gain the ESP without code
  changes. `PlatformId` now derives `PartialEq`/`Eq`.
- **hadris-iso:** New `SizeBreakdown::backup_gpt` estimator component covering
  the backup-GPT region appended to GPT/Hybrid images, so size estimates keep
  their never-underestimates contract for those schemes.

### Fixed

- **hadris-iso:** GPT and Hybrid images now write a complete GPT. The writer
  appends a backup-GPT region (backup entry array plus backup header) after the
  ISO data, so `alternate_lba` points at a real backup header instead of
  unwritten sectors, and both headers use the standard 128-entry array via
  `hadris-part::GptDisk` (previously 4 or 2 entries were advertised and no
  backup was written). `volume_space_size` covers the appended region, so tools
  that copy `volume_space_size` logical sectors preserve the backup GPT.
- **hadris-iso:** The GPT partition layout is now meaningful: basic-data
  partitions cover exactly the ISO data area (the previous single partition
  spanned LBA 34 to `disk_size - 34`, an extent with no valid filesystem), and
  the EFI System Partition entry covers the El Torito UEFI image's exact
  sectors. Partitions start no earlier than 512-byte LBA 64, where the ISO
  volume descriptors begin; tools that convert ISOHYBRID GPT images to MBR
  (such as `limine bios-install`) embed boot code below sector 63 and reject
  partitions starting earlier. The hybrid MBR mirrors the ESP as type `0xEF` alongside the `0x17`
  ISO partition instead of mirroring only the bogus basic-data range.
- **hadris-part:** `HybridMbrBuilder` no longer undersizes the protective MBR
  entry by one sector. The entry now covers LBA 1 through the sector before the
  first mirrored partition inclusive (33 sectors for a partition starting at
  LBA 34), and `end_chs` is consistent with `sector_count` in every branch.
- **hadris-part:** Corrected the byte values of many well-known partition-type
  `Guid` constants, which did not match their canonical GUIDs (all `FREEBSD_*`,
  `SOLARIS_*`, `NETBSD_*`, and `VMWARE_*` constants, the `LINUX_ROOT_*`,
  `LINUX_HOME`, `LINUX_SRV`, and `LINUX_LUKS` constants,
  `WINDOWS_STORAGE_SPACES`, and several `APPLE_*` constants). All constants are
  now defined from their canonical GUID strings at compile time, with the
  sources referenced in the code and a regression test asserting each
  constant's textual form.

- **hadris-fat:** Overwriting an existing file through `write_file` no longer
  leaks the file's previous cluster chain. The writer now follows and reuses
  the existing chain, and `finish()` frees any tail clusters left over when
  the new contents are shorter than the old, keeping the FAT and the FSInfo
  free-cluster count consistent. Previously every overwrite of a multi-cluster
  file orphaned all but its first cluster, eventually failing with
  `NoFreeSpace` (#90).
- **hadris-fat:** `create_dir` no longer leaks a cluster when it fails after
  allocating the directory's data cluster (e.g. `DirectoryFull` on a full
  fixed root, or an over-long name). The data cluster is now allocated only
  after the fallible name/slot steps succeed, matching `create_file`.
- **hadris-fat:** `FileWriter::new_append` no longer overwrites the last
  cluster of a file whose size is an exact multiple of the cluster size. The
  writer now positions past the full final cluster so appended data extends
  the file instead of clobbering its tail.
- **hadris-fat:** Stale `FileEntry` handles can no longer corrupt an unrelated
  file. If a file is deleted and its directory slot reused, operating through
  the old handle previously freed the new file's chain, cleared its
  `DIRECTORY` bit, moved it, or clobbered its entry. `delete`, `rename`,
  `truncate`, `set_attributes`, `set_times`, and `FileWriter::finish` now
  revalidate the slot's on-disk short name, creation timestamp, and
  not-deleted marker before acting and return `Error::StaleEntry` on mismatch.
  FAT carries no per-entry identity, so this is best-effort: a slot re-taken by
  a same-named file is distinguished only by the creation timestamp, which has
  10 ms resolution and is constant under the `no_std` `EpochTimeProvider`.
  Passing `created` to `set_times` rewrites that field and therefore
  invalidates other handles to the same file. `delete` also marks the
  entry deleted before freeing its chain so a mid-operation error cannot leave
  a live entry pointing at freed clusters.
- **hadris-fat:** A `FileReader` held across a delete and cluster reuse no
  longer discloses the reusing file's data. The reader revalidates its
  directory slot before serving bytes and returns `Error::StaleEntry` instead.
- **hadris-fat:** `create_file` and `create_dir` through a stale `FatDir` (its
  directory was deleted and the cluster reused) no longer write directory
  entries into an unrelated file's data. Both revalidate the directory's own
  entry first and return `Error::StaleEntry` on mismatch. The root directory,
  which cannot be deleted, is always accepted.
- **hadris-fat:** `FileWriter::finish` now reports I/O and FAT-update failures
  instead of masking them. With the `dirty-file-panic` feature, an error
  returned part-way through `finish` used to trip the drop guard on the way
  out, panicking about a writer that had in fact been finished. The guard now
  fires only for a genuinely forgotten `finish()`.
- **hadris-fat:** Opening two `FileWriter`s for the same file no longer lets
  them independently allocate and cross-link one cluster chain (the last
  `finish()` won and orphaned the other writer's clusters). The volume now
  tracks the directory slot of each open writer and returns
  `Error::WriterConflict` for a second writer on the same entry; the slot is
  released when the writer is finished or dropped.
- **hadris-fat (`unstable-exfat`):** Extending a file across a cluster boundary
  onto a fragmented layout now writes the FAT chain links, so data past the
  first cluster is reachable on read-back. The writer linearizes a contiguous
  file's prefix into the FAT when it first becomes fragmented and links each
  appended cluster onto the tail.
- **hadris-fat (`unstable-exfat`):** `allocate_clusters` is now authoritative
  against the allocation bitmap. Its fragmented fallback previously scanned the
  FAT for zero entries and re-handed-out clusters the contiguous (bitmap-only)
  path had already allocated; it now reserves clusters from the bitmap and
  links them in the FAT, rolling the reservations back if either step fails
  and returning `NoFreeSpace` when the volume is full.
- **hadris-fat (`unstable-exfat`):** `create_dir`, `delete`, and `truncate` now
  flush the allocation bitmap. Previously only the file writer persisted it, so
  a directory's allocation or a freed chain was lost on remount.
- **hadris-fat (`unstable-exfat`):** Truncating a fragmented file now frees the
  dropped tail clusters in the allocation bitmap, not only in the FAT, so the
  space is reclaimed and the bitmap and FAT stay consistent.
- **hadris-fat (`unstable-exfat`):** Directory entry sets are no longer placed
  across a cluster boundary (they were written linearly, which is wrong for a
  fragmented directory), and scanning a directory with an unknown size no longer
  walks past its allocation into unrelated data.
- **hadris-fat (`unstable-exfat`):** Overwriting a contiguous, multi-cluster
  file in place now reuses the file's own already-allocated clusters. Crossing a
  cluster boundary during an overwrite previously saw the file's next cluster as
  "already allocated" and allocated a brand-new cluster instead, orphaning the
  old one in the bitmap and needlessly fragmenting the file (the #90 pattern for
  the exFAT path).
- **hadris-fat (`unstable-exfat`):** exFAT formatting computes its cluster-heap
  geometry in 64-bit arithmetic and range-checks the results. A volume larger
  than ~2 TiB previously truncated the sector count to `u32`, silently sizing
  the filesystem to a fraction of the device; oversized geometry now returns
  `VolumeTooLarge` instead of corrupting the layout.
- **hadris-udf:** dstring decoding no longer includes one trailing garbage byte
  (usually a NUL). The dstring length byte counts the compression-ID byte, so
  `PrimaryVolumeDescriptor::volume_id()`, `LogicalVolumeDescriptor::volume_id()`,
  and `FileSetDescriptor::file_set_id()` previously returned values like
  `"LABEL\0"` and equality comparisons against the written label failed. The
  decoder also guards hostile length bytes without panicking.
- **hadris-udf:** 16-bit (UTF-16) dstrings and filenames are now decoded with
  proper surrogate-pair handling. Characters outside the Basic Multilingual
  Plane were previously dropped from names, so files written with such names
  could not be listed or found by name. Unpaired surrogates decode to U+FFFD.
- **hadris-cd:** Creating an image without a name-preserving namespace (Joliet
  disabled and Rock Ridge off) no longer fails with `ISO writer did not produce
  the planned file` for names the ISO 9660 charset sanitizes, and no longer
  produces images whose ISO and UDF namespaces diverge for sanitized directory
  names. The writer now maps ISO names onto the shared tree before writing and
  deduplicates them within each directory.
- **hadris-cd:** Rock Ridge metadata requested with `rock_ridge` creation
  options or the CLI's `-R` flag is now written. The base ISO namespace did not
  advertise RRIP support, so hadris-iso silently skipped the system-use fields.
- **hadris-cd:** The CLI's `verify` command now compares Rock Ridge alternate
  names when present, so `-R` images verify cleanly.
- **hadris-iso:** `PrimaryVolumeDescriptor::new` and
  `SupplementaryVolumeDescriptor::new_evd` no longer panic on volume names
  longer than 32 characters. The constructors truncate or substitute invalid
  input, while the writer rejects an over-long `volume_name` with
  `InvalidInput`.
- **hadris-io:** Converting `ErrorKind::UnexpectedEof` and
  `ErrorKind::WouldBlock` to `std::io::ErrorKind` now preserves the kind instead
  of mapping both to `Other` through the embedded-io conversion.
- **hadris-io:** `Cursor::seek` uses checked position arithmetic. Overflowing
  seeks return `InvalidInput`, and `SeekFrom::Start` accepts offsets above
  `i64::MAX`, matching `std::io::Cursor`.
- **hadris-storage:** Building with neither the `sync` nor `async` feature no
  longer emits dead-code warnings.
- **tools:** All five CLI tools reset `SIGPIPE` to the default disposition on
  Unix, so piping output into commands such as `head` exits without a
  broken-pipe panic.
- **hadris-udf-cli:** `create --revision` now accepts only the supported UDF
  revisions: 1.02, 1.50, 2.00, 2.01, 2.50, and 2.60.

### Documentation

- **hadris-io:** Corrected the claim that the I/O traits re-export from
  `std::io` under the `std` feature. They are Hadris traits with blanket impls
  for standard-library types. The docs now mark `alloc` as a no-op, and the
  package no longer contains the unused `traits.rs`.
- **hadris-cd:** The README quick start now opens the output file for reading
  and writing, as required by `OpticalImageWriter::finish`. The runtime error
  for a write-only target now states that requirement.
- **hadris-fixed:** Documented that `From<&[u8]>` for `FixedBytes` panics on
  over-long slices and that `try_from_slice` is the fallible alternative.
- **hadris-common:** Corrected the `sync` feature's default in the crate-level
  feature table.

## [2.1.0] - 2026-08-18

### Added

- **hadris-fat:** Directory entries and parse results now implement `Clone`,
  allowing parsed metadata to be retained independently of directory iterators.
  ([@stlankes](https://github.com/stlankes),
  [#84](https://github.com/hxyulin/hadris/pull/84))
- **Release automation:** Added a manually triggered GitHub Actions workflow
  that validates and publishes the workspace crates in dependency order, tags
  the release, and creates GitHub release notes from this changelog entry.

### Changed

- **hadris-fat:** `TimeProvider` and `OemCpConverter` now require `Sync`, which
  allows a `FatVolume` that uses them to be moved into a mutex and shared across
  threads. ([@stlankes](https://github.com/stlankes),
  [#83](https://github.com/hxyulin/hadris/pull/83))

### Fixed

- **hadris-common:** Replaced the GPL-licensed `noalloc` dependency with a
  compatibility layer backed by `heapless` while preserving the existing
  fixed-capacity collection API.
- **Build:** Added a `cargo-deny` CI gate that rejects dependencies outside the
  workspace's permissive-license allowlist.
- **hadris-fat:** Rejected FAT32 images whose BPB root cluster is outside the
  data-cluster range at mount time, and treated directory entries whose first
  cluster is below 2 as empty, fixing `attempt to subtract with overflow`
  panics on corrupt images (found by `cargo fuzz run fat_read`).
- **hadris-fat:** Short (8.3) names containing OEM high bytes no longer panic
  `FileEntry::name`, `ShortFileName::matches`, or the `Debug` impl on
  untrusted images; such names are decoded lossily, and a non-panicking
  `ShortFileName::try_as_str` was added.
- **hadris-fat:** `FileReader::read_to_vec` no longer pre-allocates the full
  claimed file size, so a corrupt entry advertising gigabytes on a tiny image
  cannot force an out-of-memory abort.
- **hadris-part:** Partition end-LBA and size computations now saturate
  instead of panicking on corrupt MBR/GPT entries whose start + size exceeds
  the integer range (found by the new `part_read` fuzz target).
- **hadris-part:** GPT reads bound the untrusted partition-entry array
  (entry count and LBAs) against the actual image size before allocating or
  seeking, returning `DiskTooSmall`/`BackupHeaderIo` instead of panicking on
  multiply overflow or allocating hundreds of GiB.
- **hadris-fat (exFAT):** Boot-sector validation no longer panics on shift
  fields whose sum overflows `u8`, and cluster arithmetic (`cluster_to_offset`,
  `is_valid_cluster`, FAT table bounds) saturates instead of over- or
  underflowing on corrupt boot-region values (found by the new `exfat_read`
  fuzz target).
- **hadris-ntfs:** Mount-time record sizes (MFT/index) and per-stream sizes
  (`$BITMAP`, index allocation blocks, `FileReader::read_to_vec`) derived
  from untrusted on-disk fields are now bounded against the actual data
  source or volume capacity, so a corrupt boot sector or attribute cannot
  force an out-of-memory abort (found by `cargo fuzz run ntfs_read`).
- **hadris-ntfs:** Mount rejects a `$UpCase` stream whose declared size is
  not exactly the 128 KiB table before reading it, so a corrupt attribute
  claiming gigabytes (backed by a huge claimed volume) cannot force an
  out-of-memory abort (found by `cargo fuzz run ntfs_read`).
- **hadris-fat:** An LFN entry with the last-entry flag but a sequence count
  of zero no longer starts a long-name sequence, fixing an
  `attempt to subtract with overflow` panic in the sequence countdown
  (found by `cargo fuzz run fat_read`).
- **hadris-fat:** FAT12/16 fixed-root directory iteration no longer stops
  after the first 4 KiB window, which silently dropped entries past slot 128
  of larger roots (e.g. the standard 224-entry floppy root) and could make
  `find`/`open_path` miss existing files (found by manual audit).
- **hadris-fat (exFAT):** Mount rejects an allocation bitmap whose claimed
  `data_length` exceeds the cluster heap, directory iteration and file
  reads/seeks return `ClusterLoop` on cyclic FAT chains instead of looping
  forever, and allocation-bitmap cluster arithmetic saturates instead of
  over- or underflowing (found by manual audit).
- **hadris-fat (tool):** `scan_fat`/`verify` no longer pre-allocate from
  claimed BPB geometry, and the recursive directory walks cap nesting depth
  with a `CorruptFilesystem` error instead of overflowing the stack on cyclic
  directory graphs (found by manual audit).
- **hadris-ntfs:** `attr::parse_index_entries` validates a caller-supplied
  node-header offset with checked arithmetic and returns
  `InvalidIndexEntry` instead of panicking on out-of-range values
  (found by manual audit).
- **hadris-part:** `PartitionInfo::size_bytes`, CHS-to-LBA conversion,
  hybrid-MBR building, `GptDisk` geometry helpers, and the GPT write path
  now use checked or saturating arithmetic, fixing divide-by-zero and
  overflow panics on crafted tables — and, in release builds, potential
  writes to wrapped offsets. Reading a GPT with `block_size` below the
  header size returns `InvalidBlockSize`. The GPT fuzz-regression tests are
  now gated off the `crc` feature, which rejects the crafted images earlier
  with a checksum error (found by manual audit).
- **hadris-iso:** `IsoDir::read_entries` bounds a directory's claimed size
  against the actual image before allocating, matching `read_file`.
  `IsoModifier::open` returns an error on images without a primary volume
  descriptor instead of panicking, rejects cyclic directory graphs (depth
  cap plus extent-visit tracking) instead of overflowing the stack, and
  skips version-only (`;1`) directory names instead of panicking in
  `finish()` (found by manual audit).
- **hadris-iso:** Rock Ridge continuation areas are now written after the
  directory records that reference them, directory extents are assigned
  pre-order (parents before children), and file data is written after the
  whole directory region. Streaming readers only follow CE pointers to
  later positions and ignore directories whose extent is lower than their
  scan position, so libarchive/bsdtar previously rejected hadris-written
  Rock Ridge images outright with "Invalid parameter in SUSP CE
  extension" and still could not list nested directories once the CE
  placement was fixed. bsdtar now lists hadris-written images cleanly.
  The writer plans the full layout (sizes are extent-independent) before
  writing, so emitted images are deterministic and the floor set by
  `create_with_allocation_floor` still reserves the low sectors.
- **hadris-iso:** `pad_align_sector` now zero-fills padding instead of
  seeking past it, so emitted bytes no longer depend on the target
  reading unwritten regions back as zeros (reused buffers, block
  devices). Output on fresh files and memory cursors is unchanged.
- **hadris-udf:** `UdfWriter::write_file_entry` returns
  `TooManyAllocationDescriptors` instead of panicking when the descriptors
  exceed a file entry, and `UdfWriter::create` rejects directory trees
  deeper than 128 levels with `DirectoryNestingTooDeep` instead of
  overflowing the stack (found by manual audit).
- **hadris-udf:** Allocation descriptors and file-identifier ICBs are now
  parsed without requiring pointer alignment, so a corrupt File Entry with an
  odd `extended_attributes_length` (or FIDs at unaligned offsets) no longer
  panics inside bytemuck on the misaligned cast (found by
  `cargo fuzz run udf_read`).
- **hadris-cpio:** `read_entry_data` returns `Error::BufferSizeMismatch`
  before consuming any bytes instead of panicking when the caller's buffer
  does not match the entry's claimed file size (found by manual audit).
- **hadris-fat:** Directory enumeration now skips any entry carrying the
  `VOLUME_ID` attribute bit (except exact LFN components), so a corrupt
  entry combining `VOLUME_ID` with `DIRECTORY` is no longer listed and
  recursed into as a directory, matching the FAT spec and mtools (found by
  differential fuzzing against mdir).

## [2.0.0] - 2026-08-09

First stable release of the V2 API. This entry consolidates the changes made
across the `2.0.0-rc.4` candidate and the subsequent specification-conformance
work; the public API frozen during the release-candidate series is now stable
under Semantic Versioning.

### Added

- **hadris-ntfs:** Added an experimental, read-only NTFS leaf crate with
  validated boot geometry, MFT and directory traversal, resident/non-resident
  file reading, sparse runs, Unicode names, and sync/async `no_std` support.
  ([@aruiz](https://github.com/aruiz))
- **Facade crates:** Added package READMEs for `hadris-archive`,
  `hadris-block`, and `hadris-optical`.
- **Specification compliance:** Added a compliance catalog framework with
  pinned source digests and extracted requirement sets for ECMA-119, ECMA-167,
  UDF 1.02, the ECMA TR/71 bridge format, and source-bounded NTFS MFT
  behavior.

### Fixed

- **hadris-iso:** Enforced ECMA-119 invariants and validated descriptor
  conformance; the aligned image tail is now preserved.
- **hadris-udf:** Corrected anchor and Volume Descriptor Sequence layout, and
  made descriptor validation and decoding portable across targets.
- **hadris-cd:** Conformed the UDF bridge descriptor layout and reserved space
  for the trailing UDF anchor.
- **hadris-cpio:** Enforced `newc` archive invariants and accepted aligned
  trailerless archives.
- **hadris-fat:** Validated BPB geometry, enforced filesystem integrity rules,
  and supported checksum validation across read tiers.
- **hadris-part:** Hardened partition metadata handling.
- **hadris-iso-cli:** Extension filenames are displayed correctly and filename
  namespace semantics are respected.

### Changed

- **Build / MSRV:** The workspace now uses Cargo resolver 3 so dependency
  resolution prefers releases compatible with the declared Rust 1.88 MSRV.
- **Documentation:** Removed superseded V2 planning, migration, performance,
  and internal agent-design notes; consolidated the active specification
  annotation rules in `docs/spec-coverage.md`.
- **hadris-fat:** Expanded cache documentation with the builder, transparent
  routing, explicit flush, capacity, and sync-only behavior.
- **Specification compliance:** Full claims now require runnable test evidence;
  CI checks evidence existence and bidirectional coverage-table parity.
- **hadris-ntfs:** Public API documentation is now enforced with
  `deny(missing_docs)`.
- **hadris-cpio:** The optional parser is now crate-internal.

## [2.0.0-rc.3] - 2026-07-22

### Added

- **hadris-iso:** Added an explicit ISO interchange `BaseIsoLevel::Level3` and
  corrected the CLI `--level 3` mapping.
- **hadris-iso:** Allocation-free readers now discover and explicitly select
  the ISO 9660:1999 enhanced namespace, including its root directory.

### Fixed

- **hadris-udf:** Directory FID extents are planned from exact encoded record
  lengths instead of an unsafe per-entry estimate.
- **hadris-udf:** OSTA CS0 filenames now select compression ID 8 or 16 from
  their Unicode contents, enforce the 255-byte encoded limit, and decode
  8-bit values as one-byte Unicode code points.
- **hadris-iso:** `has_evd()` now reports an ISO 9660:1999 Enhanced Volume
  Descriptor rather than implying that UDF is present.

### Changed

- **hadris-io / hadris-common:** `std` no longer activates `sync`; hosted
  support and I/O mode selection are independent.
- Broken Markdown links now fail the documentation build.

## [2.0.0-rc.2] - 2026-07-19

### Added

- **hadris-iso:** Added a zero-allocation `IsoReader` for sync and async
  `no_std` builds, including ISO 9660/Joliet namespace selection, nested path
  lookup, caller-buffered reads, and multi-extent file streaming.
- **hadris-fat:** Added `NtCaseFlags` and `FileEntry::nt_case` /
  `ShortFileName::with_nt_case` so lowercase 8.3 short names round-trip in their
  original case.
- **hadris-iso:** `EmulationType` now names the El Torito boot media types
  (1.2/1.44/2.88 MB floppy and hard-disk emulation) with an `is_emulated`
  helper, so bootable images can request emulated media. Emulated boot entries
  default their load size to one virtual sector.
- **hadris-iso:** Rock Ridge `TF` timestamp entries now include the creation
  time when the input entry carries one (new `RripBuilder::add_tf`), alongside
  the existing modify and access times.
- **hadris-fat:** `Fat12` and `Fat16` now expose `allocate_chain` and
  `extend_chain`, matching `Fat32`, for callers doing manual multi-cluster
  allocation.
- **hadris-iso:** The allocation-free `IsoReader` can now resolve Rock Ridge
  alternate names: `IsoDirEntry::rrip_name_into` decodes the `NM` field into a
  caller buffer and `rrip_name_matches` compares it, both without allocating.
  (Inline system-use areas only; `CE`-continued names are not followed.)

### Removed

- **hadris-iso:** Removed the unused, always-empty `read::SupportedFeatures`
  bitflags stub.

### Fixed

- **hadris-iso:** `IsoImage::open` now rejects images that declare a logical
  block size other than 2048 with a clear `Unsupported` error, instead of
  silently misreading their extents. (The allocation-free `IsoReader` honors the
  declared block size; full non-2048 support in `IsoImage` remains future work.)
- **hadris-iso:** Joliet file identifiers are now encoded as conformant UCS-2:
  characters outside the Basic Multilingual Plane are substituted with `_`
  instead of leaking UTF-16 surrogate pairs into the field, and names are capped
  at the Joliet 64-character limit (previously up to 103). `encode_joliet_name`
  is likewise BMP-safe.
- **hadris-iso:** Directory records are now written in ascending File Identifier
  order (ECMA-119 9.3) instead of input/tree order, so images validate against
  strict readers.
- **hadris-iso:** Writing a file larger than 4 GiB now fails with a clear error
  instead of silently truncating its length to 32 bits. Multi-extent records
  (which would lift the limit) are not yet emitted.
- **hadris-fat:** Lowercase 8.3 names (e.g. `readme.txt`) are now stored as a
  single short entry with the Windows NT `DIR_NTRes` case flags and read back in
  their original case, instead of being uppercased on read or spending a
  long-file-name entry. `rename` now records case flags for the new name rather
  than carrying over the source entry's.

### Documentation

- Expanded the `@hadris-spec` compliance annotations and `docs/spec-coverage.md`
  to cover more ISO volume descriptors, path tables, and El Torito section
  entries, the FAT FSInfo sector, and a new `hadris-part` (MBR/GPT) section.
- Completed the in-repo ISO 9660 specification notes (volume descriptors, path
  table, directory record, and an extensions index) and documented the ISO
  writer's known limitations.
- Recorded caching and performance findings deferred out of 2.0.
- Added a Docusaurus documentation site with getting-started, crate-selection,
  migration, release-candidate, and task-oriented FAT, partition, ISO, CPIO,
  and `no_std` guides.
- Added GitHub Pages build and deployment automation for the documentation
  site.
- Added runnable workspace examples for listing FAT images and partition
  tables, detecting optical formats, and creating CPIO archives.

### Changed

- Removed the unused top-level `tests` and `resources` placeholders; tests and
  fixtures remain colocated with their owning crates.

## [2.0.0-rc.1] - 2026-07-16

### Added

- **hadris-cd-cli:** New `hadris-cd` utility for creating, inspecting, and
  verifying ISO 9660/UDF bridge images, including Joliet, Rock Ridge, El Torito,
  and hybrid MBR/GPT options.
- **hadris-cd / hadris-iso:** Direct non-empty bridge qualification through
  both concrete readers and an ISO allocation-floor API for collision-free
  composition with other on-disc metadata.
- **hadris-fat-cli:** `cat`, selective and recursive extraction, and recursive
  FAT image creation with automatic or explicit sizing.
- **hadris-part:** `read` is now a default feature; I/O extension traits
  (`MasterBootRecordReadExt`, `GptDiskReadExt`, `DiskPartitionSchemeReadExt`,
  and write counterparts) are re-exported at the crate root.
- **hadris-part:** Explicit `crc` / `rand` feature flags; docs.rs builds with
  all features.
- **hadris-part:** I/O roundtrip integration tests for MBR read/write and
  scheme detection.
- **hadris-macros:** Dual sync/async integration guide in the crate README.
- **CI:** `check-features` tiers for `hadris-io` async and `hadris-part`
  async-read / crc.
- **hadris-udf:** Public `UdfFs::read_file`; directory listings populate
  `UdfDirEntry::size` from each file ICB.
- **hadris-udf-cli:** `cat` and `extract` subcommands.
- **Async integration coverage:** Direct leaf-level runtime tests for FAT
  traversal and multi-cluster reads, GPT detection/opening, ISO descriptor and
  file reads, and UDF nested traversal/file reads.

### Changed

- **CLI tools:** Canonical installed binaries now form the `hadris-fat`,
  `hadris-iso`, `hadris-udf`, and `hadris-cpio` family. Existing executable
  names remain compatibility aliases, and CPIO standardizes on `ls` with
  `list` retained as an alias.
- **Workspace cleanup:** Removed the unpublished `hadris-cli` FAT debug stub;
  the supported V2 command-line surface is the specialized `hadris-*` family.
- **Project positioning and package metadata:** Reframed Hadris as a layered
  Rust storage stack, documented its architecture and target environments, and
  refreshed the `hadris`, FAT, ISO, UDF, block, and storage crate descriptions
  and search keywords.
- **Public API documentation:** Completed and now enforce missing-doc coverage
  for the fixed-capacity, I/O, common, storage, block facade, optical facade,
  FAT, partition, ISO, UDF, CPIO, and hybrid optical writer crates.
- **Release process:** Removed shared workspace versioning and the obsolete
  `cargo-release` configuration. Every current package now declares version
  `2.0.0-rc.1` in its own manifest.

- **hadris-part:** `PartitionError::Io` now wraps `hadris_io::Error` (with
  `std::error::Error::source` under `std`) instead of discarding context.
- **hadris-cd:** Missing/unreadable source paths during ISO tree conversion
  now return `CdError` instead of silently writing empty files.
- **hadris-iso / hadris-fat / hadris-udf:** Documented known limitations in
  crate-level rustdoc.
- **CI / process:** MSRV pinned to Rust 1.88.0 (required for `let`-chains in
  `hadris-macros`; `rust-toolchain.toml`, workspace `rust-version`, CI
  toolchain); CLI `--help` smoke job; workspace `cargo doc` job; Dependabot
  for Cargo and GitHub Actions; CONTRIBUTING.md. Fuzz harnesses remain
  local-only (not PR CI).

## [1.2.1] - 2026-07-09

### Added

- **Fuzzing:** Coverage-guided fuzz harnesses for `cpio_read`, `fat_read`,
  `iso_read`, and `udf_read`, with a committed seed corpus (including CPIO
  allocation-DoS regressions).
- **SECURITY.md:** Project security policy.

### Fixed

- **hadris-fat / hadris-iso / hadris-udf / hadris-cpio:** Bound untrusted length
  fields before allocating; reject inputs that previously panicked readers.
- **hadris-udf:** Validate File Entry allocation window before slicing.
- **hadris-fat:** Skip volume label entries when listing directories.

### Documentation

- Workspace and crate READMEs updated for API accuracy and version `1.2.1`.

## [1.2.0] - 2026-06-13

### Added

- **hadris-fat:** `FatFs::builder` / `FatFsBuilder` for configuring a volume
  before mount, with pluggable providers:
  - `with_time_provider` — custom clock (`TimeProvider`) for directory-entry
    timestamps.
  - `with_oem_converter` — custom OEM codepage (`OemCpConverter`) for short
    (8.3) filename encoding.
  - `with_fat_cache` — optional LRU FAT-sector cache (requires the `cache`
    feature; sync API only).
- **hadris-fat:** Long filename (VFAT/LFN) **write** support — create and
  delete entries with names up to the 255 UTF-16 code-unit spec cap, including
  supplementary-plane (surrogate-pair) characters.
- **hadris-fat:** Volume timestamp, label, and status-flag (`dirty`,
  `io_errors`) read APIs, sourced from the FAT-resident status word (`FAT[1]`).
- **hadris-fat:** `cache` feature — LRU FAT-sector cache reducing redundant
  seek+read I/O for FAT entry access; dirty entries flush to all FAT copies on
  eviction. `cache` implies `sync`.
- **hadris-fat:** `defmt` support for error types in embedded/no-std contexts.
- **CI/safety:** Miri jobs covering historically-unsafe code paths — LFN
  UTF-16 / surrogate-pair handling and LFN write encoding / union access.

### Changed

- **hadris-fat:** Errors now carry I/O context (`IoContext`) describing the
  failed operation instead of a bare I/O error.
- **hadris-part:** MBR LBA and GPT on-disk fields use `endian-num` typed
  endian fields instead of manual byte handling.
  ([@aruiz](https://github.com/aruiz),
  [#21](https://github.com/hxyulin/hadris/pull/21))
- **Workspace:** endianness types moved to `zerocopy`; `alloc` error types
  supported in no-std builds.

### Fixed

- **Soundness:** Eliminated UTF-8 undefined behavior when converting disk
  bytes to `&str` in LFN, `IsoStr`, and `IsoString`; removed unsoundness in
  `FixedFilename::as_str`.
- **hadris-fat:** Guard against infinite loops on corrupt cluster chains
  (cluster-loop / out-of-bounds / bad-cluster markers now return errors).
- **hadris-fat:** FAT32 `FSInfo` was not flushed on some write paths.
- **hadris-udf:** Fixed errors when parsing Windows 11 ISO images.
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#29](https://github.com/hxyulin/hadris/pull/29))
- **hadris-iso:** Auto-convert lowercase in PVD string fields instead of
  panicking.
- **Docs:** Fixed broken rustdoc intra-doc links (`OemCpConverter`, `FAT[1]`);
  docs.rs builds `hadris-fat` with the full stable sync feature set while the
  unstable exFAT preview remains opt-in.
- **hadris-fat:** The `tool` feature now implies `sync` and is emitted only
  in the sync slice — the analysis/verify utilities iterate directories
  synchronously, so `--features async,tool` previously failed to compile.
- **hadris-fat:** All sync-only cache code (`with_cached_fat`,
  `with_fat_cache_locked`, `fat_cache`, internal `*_via_cache` helpers) is
  now confined to the sync slice, so `--features async,cache` and
  `--all-features` compile (the cache is simply bypassed under async).
- **hadris-fat:** Long-filename entry runs may now cross directory cluster
  boundaries, including maximum-length names and directory extension.

### Known limitations

- **async + cache:** The FAT-sector cache is sync-only. Driving a volume
  through the async API silently bypasses the cache (async-aware caching is
  deferred — see the `cache` feature note in `hadris-fat/Cargo.toml`).
- **exFAT:** Available only as the leaf-crate `unstable-exfat` preview. It is
  outside the V2 API stability promise and unified block opener; fragmented
  system metadata, directory growth/general cross-cluster entry placement,
  async operation, TexFAT, and repair workflows remain unsupported.

## [1.1.0] - 2026-03-12

### Added

- **hadris-fat:** Added configurable FAT table readahead and reduced fragmented
  file reads to one I/O operation when `FileReader` uses a cached chain.
  ([@aruiz](https://github.com/aruiz),
  [#13](https://github.com/hxyulin/hadris/pull/13),
  [#15](https://github.com/hxyulin/hadris/pull/15))

### Fixed

- **hadris-fat:** Corrected FAT32 entry endianness and fixed cached builds to use
  the volume's cluster bounds. ([@aruiz](https://github.com/aruiz),
  [#11](https://github.com/hxyulin/hadris/pull/11),
  [#12](https://github.com/hxyulin/hadris/pull/12))
- **Build:** Disabled `thiserror` default features so the workspace builds as
  `no_std`. ([@aruiz](https://github.com/aruiz))

[Unreleased]: https://github.com/hxyulin/hadris/compare/v2.4.0...HEAD
[2.4.0]: https://github.com/hxyulin/hadris/compare/v2.3.0...v2.4.0
[2.3.0]: https://github.com/hxyulin/hadris/compare/v2.2.0...v2.3.0
[2.2.0]: https://github.com/hxyulin/hadris/compare/v2.1.0...v2.2.0
[2.1.0]: https://github.com/hxyulin/hadris/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.4...v2.0.0
[2.0.0-rc.3]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.2...v2.0.0-rc.3
[2.0.0-rc.2]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.1...v2.0.0-rc.2
[2.0.0-rc.1]: https://github.com/hxyulin/hadris/compare/v1.2.1...v2.0.0-rc.1
[1.2.1]: https://github.com/hxyulin/hadris/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hxyulin/hadris/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/hxyulin/hadris/releases/tag/v1.1.0

# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each published package owns its version and may be released independently.

## [Unreleased]

### Added

- **hadris-fat (V3):** The embedded API for firmware without an
  allocator: `hadris_fat::embedded::sync::Fat<D, const FILES: usize = 4>`
  and its `embedded::r#async` twin over a
  `hadris_storage::local::BlockDevice`, built on `hadris-fat-raw` rather
  than `FatFs`. It reads and writes FAT12, FAT16 and FAT32 through one
  512-byte block buffer and `FILES` file slots, under 1 KiB of state with
  four slots, and takes 512-byte device blocks only. `Dir` is a `Copy`
  directory handle, `File` a slot consumed by `close` whose generation
  makes a stale or foreign handle fail with `InvalidHandle`, and `list`
  lends each `Entry` to a callback; `Entry::node` with `open_node` opens a
  listed file without a lookup. `Options` sets a `fn() -> DateTime` clock,
  the name fold (`fold_ascii` by default, `fold_unicode` as an opt-in), the
  UTC offset, the code page and read-only. It has `open`, `read`, `write`,
  `seek`, `set_len`, `flush`, `close`, `open_dir`, `create_dir`,
  `create_dir_all`, `metadata`, `set_attr`, `remove_file`, `remove_dir`,
  `remove_dir_all` (without a stack, at most 1024 levels), `rename`,
  `label`, `stats`, `was_dirty`, `sync` and `unmount`. Writes keep
  `FatFs`'s crash ordering, and the next writing call or `sync` frees what
  an interrupted operation or a dropped future held.
- **CI (V3):** A `cross` job builds the `no_std` tiers for
  `thumbv6m-none-eabi`, `thumbv7em-none-eabihf` and
  `riscv32imc-unknown-none-elf` through `scripts/check-targets.sh`; the
  targets without compare-and-swap skip the `alloc` tiers.
- **hadris-fs (V3):** `Walk` in each mode lists everything below a
  directory depth first on the bare driver: `Walk::new(dir)` on a heap
  stack of up to 1024 levels (`alloc`), or `Walk::with_stack(dir, &mut
  [WalkFrame])` on the caller's stack without allocating. `next(&mut fs)`
  returns a `WalkEntry` with the directory entry and its depth, and
  `skip_dir` stays out of the directory just returned. A directory loop
  fails with `Corrupt`, a walk deeper than its stack with `LimitExceeded`,
  and the walk fuses after an error. `Extent` gains `file_offset` and an
  `unwritten` flag (`with_file_offset`, `with_unwritten`, `is_unwritten`)
  for file maps.
- **hadris-fat (V3):** Format extras on `FatFs` and `ExFatFs`: `info()`
  returns the boot sector's `Geometry`, `was_dirty()` whether the volume
  was not cleanly unmounted (FAT entry 1's clean bit, exFAT's
  `VolumeDirty`), `extents(node, from, &mut [Extent])` maps a file or
  directory to device ranges FIEMAP style, with exFAT bytes past
  `ValidDataLength` marked unwritten, `records(node, &mut [Extent])`
  locates its directory entries, `read_raw(offset, buf)` reads the device
  through the driver, and `set_volume_serial(serial)` rewrites the serial
  (both exFAT boot regions with their checksums, and the FAT32 backup boot
  sector). `FatFs::set_label` writes, creates or removes the root label
  entry and the boot sector copy.
- **hadris-iso (V3):** Extras on `IsoFs`: `info()` returns `VolumeInfo`
  (`block_size`, `volume_space_size`, `id(IsoId)` as stored bytes and
  `date(IsoDate)`), `boot_catalog(&mut buf)` reads and checks the El Torito
  catalog into the caller's buffer without an allocator and returns a
  borrowing `BootCatalog` whose `entries()` are parsed as iterated,
  `boot_image(&entry)` locates an entry's image, `records(node, &mut
  [Extent])` locates a node's directory records, and `extents(node, from,
  &mut [Extent])` maps its data with file offsets. `IsoId` and `IsoDate`
  no longer need `alloc`.
- **hadris-udf (V3):** Extras on `UdfFs`: `info()` returns `VolumeInfo`
  (`revision`, `block_size`, `partitions`, `implementation`, `domain`,
  `id(UdfId)`, `recorded`, `integrity_recorded`, `volume_serial`) with
  `EntityId`, `PartitionInfo` and `PartitionKind`; `was_dirty()` reports an
  open integrity descriptor; `records` locates a file entry; `read_raw`
  reads the device; and `extents(node, from, &mut [Extent])` maps data,
  with allocated but unrecorded extents marked unwritten and embedded data
  located inside the file entry. `UdfId` no longer needs `alloc`.
- **hadris (V3):** `hadris::sync::detect` and `hadris::r#async::detect`
  list every format a device holds as a `Detection` of `Candidate`s, most
  specific first and without allocating: `ImageFormat::Fat(FatKind)`,
  `ExFat`, `Iso`, `Udf`, `IsoUdfBridge`, `Cpio(Format)`, `Mbr`, `Gpt` and
  `Ntfs`, each with the `Corrupt` error a mount would give when its
  signature is present but its first structures are damaged. `open(dev,
  options)` mounts the first filesystem found as an `AnyFs` (`Fat`,
  `ExFat`, `Iso`, `Udf`), which implements `FileSystem`; NTFS, partition
  tables and archives fail with `NotRecognized`. `hadris::host::open(path)`
  opens an image file read-only. The new `detect` feature, on by default,
  adds these with the `udf` format.
- **hadris-fat-raw:** `Geometry::volume_serial()` for FAT12/16/32 and
  `FatKind::clean_bit()`; the exFAT `Geometry::serial()` is now
  `volume_serial()`.

- **hadris-iso (V3):** Appended partitions: `Hybrid::with_appended(AppendedPartition::esp(content))`
  stores a partition after the files and lists it in the GPT, and
  `BootEntry::uefi_appended(index)` boots it from El Torito, so an EFI
  system partition is stored once (BUILD-ESP-01). The first appended
  partition is the GPT's EFI system partition.
- **hadris-iso (V3):** `IsoId` and `IsoDate` with `IsoOptions::with_id`
  and `with_date` set every descriptor identifier, the copyright, abstract
  and bibliographic file identifiers included, and every descriptor date.
- **hadris-udf (V3):** `UdfOptions::with_seed`. The volume set identifier
  starts with a 16-digit serial derived from the seed or the time and the
  tree, as UDF 2.2.2.5 asks. `UdfId` with `with_id` sets the volume, volume
  set, logical volume and file set identifiers separately.

- **hadris-fat, hadris-fat-raw (V3):** `MountOptions::backup_boot` mounts
  a FAT32 volume read-only from its backup boot sector (sector 6) and an
  exFAT volume read-only from its backup boot region, without trying the
  main one. FAT12 and FAT16 have no backup and mount as usual. The raw
  layer gains `io::{sync, r#async, local}::read_backup_geometry` and
  `exfat::io::{sync, r#async, local}::read_backup_boot`.
- **hadris-udf (V3):** `MountOptions::backup_boot` reads the anchors at the
  end of the volume and the reserve volume descriptor sequence first.

- **hadris-fs (V3):** `Tree::fingerprint`, a hash of the tree's paths,
  types, sizes, link targets, device numbers and times that writers mix
  into serials and GUIDs.
- **hadris-fat (V3):** `VolumeLabel` and `exfat::VolumeLabel` implement
  `TryFrom<&str>`.

- **hadris-fat (V3):** `fat::{sync, r#async}::write(dev, &tree,
  &FatOptions)` and `exfat::{sync, r#async}::write(dev, &tree,
  &ExFatOptions)` format a device and copy a `Tree` into it with
  `copy_tree`, returning a `Report` with the volume size. Nodes without
  times get the options' time, so the same tree gives the same bytes.
  `Geometry` and `exfat::Geometry` are re-exported, since `format` returns
  them.

- **hadris-fs (V3):** The builder input and output of step R6. `Tree`
  with `insert`, `link`, `remove`, `replace`, `get`, `entry` and `root`,
  at `/`-separated byte paths that refuse `.`, `..` and NUL; `TreeEntry`
  walks it, children sorted by name bytes. `Node` is `file`, `dir`,
  `symlink` or `special`, with `with_attrs(SetAttr)`. `Content` is
  opaque and cloneable with a fixed length: `bytes`, `empty`, `stored`
  extents for session writers, host files and lazy volume content.
  `Report` (`size`, `warnings`, `extents`, `files`, `Display`) with
  `Warning` and the non-exhaustive `WarningKind` (`Renamed`, `Truncated`,
  `Deduplicated`, `Relocated`, `Dropped(Field)`, `Skipped`, `Boot`) is
  what every writer, planner and `copy_tree` returns.
- **hadris-fs (V3):** `read_tree(&vol, path)` in each mode reads a mounted
  volume into a `Tree` whose content is read lazily through the `Volume`,
  keeping hard links, symlinks and devices; `Volume::into_inner` fails
  while the tree lives. Lazy content is readable only by writers of the
  mode that produced it, others fail with `Unsupported`.
  `ContentReader::check` tells a writer so before it writes anything.
- **hadris-fs (V3):** `host` (`std` and `sync`): `read_tree(dir,
  &TreeOptions)` with `Symlinks`, `OnError`, an exclude filter, an owner
  override and an mtime clamp, returning the tree and the errors it
  skipped; `write_tree(dir, &tree)`, which never writes outside `dir`,
  creates symlinks last and reports what it drops; `file`,
  `source_date_epoch`, `mount_options` and `local_utc_offset`.
- **hadris (V3):** `hadris::host` re-exports the `hadris-fs` host
  functions and types with `FileDevice` and `StdIo`.
- **hadris-cpio (V3):** `plan`, `sync::read_tree` and `r#async::read_tree`,
  `Writer::append_file` with `EntryWriter` for data produced while
  writing, and `CpioOptions::with_time` for entries that set no
  modification time.
- **hadris-iso (V3):** `plan` outside the mode modules, and
  `IsoOptions::with_time` and `with_seed`; GPT GUIDs derive from the seed,
  or the time, and the volume identifier. `RockRidgeInfo::created`,
  `modified`, `accessed` and `changed`.
- **hadris-udf (V3):** `plan` outside the mode modules,
  `UdfOptions::with_time`, and the ISO 9660 and UDF bridge writer that
  `hadris-cd` held: `plan_bridge` and `write_bridge` in each mode, taking
  the `IsoOptions` and `UdfOptions` of the two volumes.
- **hadris-cd (V3):** `plan` outside the mode modules and
  `CdOptions::with_time`.

- **hadris-fat-raw (V3):** `short_name::display_label`, which decodes a
  stored volume label through a code page.
- **hadris-fs (V3):** `MountOptions`, one mount configuration for every
  format: `read_only`, `with_clock`, `with_utc_offset`, `with_code_page`,
  `with_node_limit` and `backup_boot`, with matching getters. The default
  is read-write, `NoClock`, UTC, CP437 and no node cap on every target.
  `CodePage` (now `Send + Sync`), `Ascii` and `Cp437` moved here from
  `hadris-fat`.
- **hadris-fat-raw (V3):** New crate, version 0.1.0: the on-disk layer of
  FAT12/16/32 and exFAT, `no_std` and allocation-free, with its own
  version. It holds the boot sector, BPB, FSInfo and directory entry
  layouts and the codecs `hadris-fat` used privately: `parse_boot` into a
  `Geometry` (with `RootLocation`, the active FAT, mirroring and the FSInfo
  sector), `FatKind` entry encoding and link classification, `ChainGuard`
  cycle detection, `Slot`, `ShortEntry` and `LongEntry`, `lfn_checksum`
  and the `lfn`, `short_name`, `name`, `date` and `layout` modules, and
  `fold_ascii` and `fold_unicode`, name folds of type `fn(u16) -> u16`.
  `exfat` holds the exFAT layouts, `parse_boot`, `boot_checksum`,
  `set_checksum`, `seal`, `name_hash`, `NameUnits`, `encode_time`,
  `decode_time` and the up-case table decoder.
- **hadris-fat-raw (V3):** `io`, the FAT device primitives the `FatFs`
  driver is built on, in `io::sync`, `io::r#async` and `io::async_send`
  behind the `sync`, `async` and `async-send` features. They borrow a
  caller-lent `BlockBuf` of one device block and a `Fat` that tracks the
  free count, the allocation hint and FAT entries an interrupted write left
  unmirrored: `read_geometry`, `read_fat`, `get`, `get_copy`, `set` (active
  copy first), `mirror`, `next`, `walk`, `run`, `allocate`, `allocate_run`
  and `free_chain` (a device block of entries at a time, recording
  progress in a `Held`), `count_free`, `write_fs_info`, `slot_offset` with
  a `DirWalk` that keeps its chain position, `read_slot`, `write_slots`,
  `clear_slots` and `mkfs`, plus `load`, `store`, `read_bytes`,
  `write_bytes` and `write_zeros`.
- **hadris-fat-raw (V3):** `exfat::io`, the exFAT device primitives the
  `ExFatFs` driver is built on, in the same three modes. An `ExFat` state
  tracks `VolumeFlags`, the Allocation Bitmaps and the free count, and an
  `Upcase` index decodes the up-case table lazily: `read_boot` (with the
  backup boot region), `read_volume`, `upcase`, FAT `get`, `next` and
  `set`, `bit`, `bitmap_bytes`, `set_bit`, `count_free`, `allocate`,
  `allocate_run` and `free_chain` (bitmap writes batched per device block,
  progress in a `Held`), `write_set` (secondary entries before the File
  entry's block) and `clear_set`, and `begin_write`, `clear_dirty` and
  `write_percent_in_use` for `VolumeDirty` and `PercentInUse`.
- **hadris-fs (V3):** `Finding`, `Severity` and `CheckReport`, what every
  checker reports. A `Finding` has a static message, the format's detail
  code, a `Severity` (`Notice`, `Warning`, `Error`, in that order; `Error`
  unless set), a `Location` and the path of the node it is about, and
  displays as `message: path (location)`. A `CheckReport` counts the
  findings and the passes over the tree.
- **hadris-fat-raw (V3):** `Detail` and `exfat::Detail`, the detail codes of
  FAT and exFAT volumes, re-exported as `hadris_fat::Detail` and
  `hadris_fat::exfat::Detail`, with `of`, `from_code` and `code`. Mount
  errors carry them: a rejected boot sector is `BootSector`, a missing or
  short Allocation Bitmap `Bitmap`, a bad up-case table entry
  `UpcaseTable`. Chain walks report `BrokenChain`, `BadCluster` and
  `CyclicChain`.
- **hadris-fat-raw (V3):** `io::{sync, r#async, async_send}::check(&mut
  dev, scratch, on_finding)`, the FAT checker, now on an unmounted device
  and allocation-free. It reports each `hadris_fs::Finding` with a
  `Detail` code, a severity, a location and the path of its entry (the
  first 1 KiB of `scratch`; the rest is the cluster bitmap, at least 512
  bytes), and returns a `hadris_fs::CheckReport`. A damaged FAT32 boot
  sector is a finding and the check goes on from the backup boot sector.
  A clear FAT16 or FAT32 clean-shutdown bit is a new `Dirty` notice.
- **hadris-fat-raw (V3):** `exfat::io::{sync, r#async, async_send}::check`,
  the exFAT checker, on an unmounted device and allocation-free like the
  FAT one, with `exfat::Detail` codes. A damaged main boot region is a
  finding and the check goes on from the backup. `exfat::io` also gains
  `DirWalk` and `slot_offset`, the directory walk `ExFatFs` now uses.
- **hadris-storage (V3):** `BlockDevice` gains `max_block_count` (how many
  blocks a device holds once written past its end, `block_count` by
  default), `disk_offset` (the byte offset of block 0 on the disk a device
  is a window of, 0 by default) and `writable` (whether the device accepts
  writes at all, false by default). `&mut D`, `Box<D>`, `Cache` and
  `Partition` forward them.
- **hadris-storage (V3):** `Partition<D>`, a byte window of a device such
  as an MBR or GPT partition, with `new(dev, offset, len)`, `offset`,
  `len`, `into_inner`, `get_ref` and `get_mut`. It is one type for every
  mode, reports `disk_offset` as the device's plus its offset, and refuses
  a request past its end, or to a window not aligned to the device blocks,
  with kind `InvalidInput` before it reaches the device.
- **hadris-storage (V3):** `host::FileDevice` (`std` and `sync`), a host
  image file or disk device with 512-byte blocks. `open(path)` opens it
  read-only; `new(file)` takes a file the caller opened and is writable
  when the file was opened for writing. Both measure the size with
  `host::file_len` and fail when a disk device cannot be measured. A
  writable image file grows when written past its end, and its
  `max_block_count` is unbounded; a read-only one refuses writes with kind
  `ReadOnly` without calling the OS.
- **hadris-storage (V3):** `Vec<u8>` is a block device with 512-byte blocks
  that grows when written past its end, filling any gap with zeros.
- **hadris-io, hadris-storage (V3):** A `local` module (with `async`) holds
  the async traits whose futures need not be `Send`, for single-threaded
  executors: `hadris_io::local::{Read, Write, Seek}` and
  `hadris_storage::local::BlockDevice`, generated from the same source as
  `r#async`. `Partition`, `MemDevice` and `Vec<u8>` implement it.

- **hadris-io (V3):** `Error<E>`, `ErrorKind`, `Location`, `DetailCode`,
  `Errno` and `FsResult` live here, the lowest crate, so block devices and
  filesystems return one error type; `hadris-fs` and the `hadris` root
  re-export them. `Error<E>` carries a kind, a static `message()`, an
  optional `location()` (byte, block, cluster or a byte of a name), an
  optional `detail()` code (a `u16` within a static domain, for a format
  crate's `Detail::of`) and the device's own error. The context is `Copy`
  and needs no allocation, and `Error<E>` is `Clone` and `Copy` when `E`
  is. Build one with `Error::new(kind, message)` or
  `Error::device(err, message)`, then `with_location` and `with_detail`.
- **hadris-io (V3):** `ErrorKind::NotRecognized`, for bytes that are not
  the format at all, apart from `Corrupt`.
- **hadris-io (V3):** `ErrorKind::errno()` returns a symbolic `Errno`, one
  per kind, and `Errno::linux()` its number. `Unsupported` is `EOPNOTSUPP`,
  `Corrupt` is `EUCLEAN` and `InvalidHandle` is `ESTALE`.
- **hadris (V3):** `Error`, `ErrorKind`, `Location`, `DetailCode`, `Errno`,
  `FsResult` and `MountError` are re-exported at the crate root.
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

- **hadris-fat-raw (V3):** `short_name::generate` no longer uppercases
  non-ASCII characters itself; the `encode` closure folds and maps them,
  so building short names links no Unicode case tables.
- **hadris-fat (V3):** `FatFs` compares names by folding each UTF-16 unit
  with `hadris_fat_raw::fold_unicode`, as Windows does, instead of folding
  characters. Letters outside the Basic Multilingual Plane now match only
  in the same case. `hadris_fat_raw::name::eq_folded` takes the fold as a
  `fn(u16) -> u16` and replaces `name::fold` and `name::eq_ignore_case`.
- **hadris-cli (V3):** One `hadris` binary replaces the five CLI crates,
  with the subcommands `fat`, `iso`, `udf`, `cpio` and `detect`. The
  bridge commands of `hadris-cd` are `hadris udf bridge` and `hadris udf
  compare`. Every format shares one set of flags and rules: `create`
  refuses an existing output unless `-f/--force` is given, `extract` never
  replaces existing files (cpio extraction now refuses them too), unreadable
  source entries are skipped with a warning, and in-image paths may start
  with `/` or `./` (cpio `cat` included). `hadris iso create` writes a
  UEFI-only boot catalog when only `--efi-boot` is given. `hadris cpio
  extract` takes `-p/--path` like the other formats and writes that entry
  or subtree to `<output>/<name>`.
- **hadris-iso (V3):** The option reshape of 5.2. `with_joliet()` and
  `with_rock_ridge()` take no argument; `with_relocation` and
  `with_preserve` are on `IsoOptions`, and `Relocation::Reject` is
  `Refuse`; `with_enhanced_tree` is `with_iso1999`. `ElTorito::new()`
  starts empty and takes entries with `with_entry`; a catalog without
  entries fails with `Detail::BootImage`. `BootEntry::bios(path)`,
  `uefi(path)` and `uefi_appended(index)` replace `BootEntry::new`,
  `with_boot_info(BootInfo::Table)` replaces
  `with_boot_info_table(BootInfo::Standard)`, and `image()` returns
  `Option<&str>`. `HybridBoot` is `Hybrid` with `mbr`, `gpt` and
  `gpt_hybrid_mbr`; `with_bootstrap` takes `&[u8]`. Identifiers are stored
  as given and only their length is checked.
- **hadris-udf (V3):** A volume identifier longer than the 30 bytes of its
  field fails with `Detail::Identifier` instead of being cut; the logical
  volume keeps 126.

- **hadris-iso (V3):** `IsoImage` and `IsoView` merge into `IsoFs`, which
  mounts like every other driver: `IsoFs::mount(dev, MountOptions)` reads
  the most capable tree (Rock Ridge, then Joliet, then the enhanced tree,
  then the primary tree), `IsoFs::mount_namespace(dev, options, namespace)`
  picks one, and `unmount` gives the device back. The image methods
  (`namespaces`, `block_size`, `volume_blocks`, `boot_catalog_block`,
  `descriptor`, `primary_descriptor`, `boot_catalog`) are on `IsoFs`, and
  `read_bytes` is `read_raw`. `view` and `into_view` are gone.
- **hadris-udf, hadris-ntfs (V3):** `UdfFs::open(dev)` and
  `NtfsFs::open(dev)` are `mount(dev, MountOptions)`, with `unmount`.
- **hadris-udf (V3):** A directory entry whose file entry is damaged is
  listed with the type its identifier records, no permissions and length
  0, instead of failing the whole listing; `stat` and `open` on it fail
  with `Corrupt`. A device without a UDF recognition sequence fails to
  mount with `NotRecognized` even when it has no anchor.

- **hadris-fs (V3):** Warning paths use the form `Tree::insert` takes and
  `Report::extents` keys: no leading, trailing or repeated `/`
  (`boot/grub.cfg`, not `/boot/grub.cfg`). `read_tree` of a single file
  names the node as its directory lists it, so on a case-insensitive
  volume the tree holds the stored spelling rather than the one in the
  path.
- **hadris-fat, hadris-iso (V3):** Volume serials written by FAT and exFAT
  `write`, and the GPT GUIDs of hybrid ISO images, derive from the tree's
  paths, sizes and times together with the seed, or the time without one,
  so different trees get different ids and the same inputs still give the
  same bytes.

- **hadris-fat (V3):** `format(&mut dev, &opts)` in each mode, FAT and
  exFAT, returns the volume's `Geometry` instead of a mounted volume, and
  needs no allocator; mount with `FatFs::mount` or `ExFatFs::mount`
  afterwards. A failed format no longer returns the device, since it is
  borrowed. `FormatOptions` is now `FatOptions` and `exfat::FormatOptions`
  `ExFatOptions`: `with_clock` became `with_time(DateTime)`,
  `with_volume_id` became `with_serial`, and `with_seed`, `with_size` and
  `with_partition_offset(bytes)` are new; FAT gains `with_alignment`.
  `with_hidden_sectors` is gone, and the FAT hidden sectors and the exFAT
  `PartitionOffset` now default to the device's `disk_offset`, so
  formatting a `Partition` records its start. A volume larger than the
  device grows a growable device and fails with `NoSpace` on any other.

- **hadris-fs (V3):** `copy_tree(&tree, &mut fs, dir)` copies a `Tree`
  into a directory of any filesystem and returns a `Report`: directory
  times are set after their children, fields the target does not store are
  reported once per field with a count, and symlinks, special files and
  extra hard-link names are skipped and reported by path. Errors carry the
  tree path. The source of every copy is a tree; copy between volumes
  with `read_tree` then `copy_tree`.
- **hadris-fs (V3):** `PathError` paths are bytes: `with_path` takes
  `impl AsRef<[u8]>` and `path` returns `Option<&[u8]>`.
- **hadris-cpio (V3):** `CpioWriter` is `Writer`. `append(path, &Node)`
  replaces `append(path, &SetMetadata, NewEntry)`, `append_hard_links`
  takes a file `Node`, and `finish` returns the stream with the `Report`.
  `write` returns the shared `Report`, whose extents give where each
  file's data starts and whose warnings count dropped fields once per
  field. Names are bytes. `Format::NewcCrc` is `Format::Crc`.
- **hadris-iso, hadris-udf, hadris-cd (V3):** Writers return the shared
  `hadris_fs::Report`, plan without I/O (content lengths are fixed when
  content is made), check every file's content is readable in their mode
  and that the device's `max_block_count` holds the image before writing,
  failing with `NoSpace` otherwise. `IsoOptions`, `UdfOptions` and
  `CdOptions` have no clock parameter: `with_time` fixes every timestamp,
  1980-01-01 by default. Warnings name paths and the stored name; names
  that are not UTF-8 are written with U+FFFD and reported as `Renamed`.
  Rock Ridge relocations are reported as `Relocated`.
- **hadris-iso (V3):** `Session` reads FIFOs and sockets into its tree;
  `Session::warnings` is gone with nothing left to report.
- **hadris-cd (V3):** The writer is `hadris_udf::write_bridge`; the UDF
  volume is checked first for the output block size.
- **hadris-fat (V3):** `FileSystem::label` decodes a FAT label through the
  mount's code page, as short names are, instead of reading a label with
  bytes above `0x7F` as `Some("")`.
- **All crates (V3):** One async mode. The `async_send` modules become
  `r#async`, whose futures are `Send` when the device is, and the former
  `r#async` modules, whose futures were not `Send`, are removed. The
  `async-send` feature is removed; `async` enables `r#async`. `hadris-io`,
  `hadris-storage` and `hadris-fat-raw` keep the futures that need not be
  `Send` in `local`, which `async` also enables. `hadris-fs` drops the
  `embassy-sync` feature and the async `Local` lock with them.
- **hadris-fs (V3):** One filesystem trait. `FileSystem` in `sync` and
  `r#async` takes `&mut self` and node ids: `lookup`, `resolve` (with a
  `Resolve` policy: `Lexical`, `Follow` or `NoFollow`) and `parent` pin a
  node, `forget(node, count)` unpins it, `readdir(dir, cursor)` returns a
  `DirEntry` that holds its name, node, metadata and next cursor, `label`
  writes the volume label into a caller buffer, `readlink` is required, and
  `open`/`close` take an `OpenMode` (a directory fails with
  `IsADirectory`). The write half (`setattr`, `write`, `truncate`, `fsync`,
  `create`, `mkdir`, `unlink`, `rmdir`, `rename` with `RenameMode`, `sync`)
  defaults to `ReadOnly`. `&mut F` and `Box<F>` forward every method.
  `Volume<F>` owns a filesystem behind a lock and shares it between threads
  and tasks with `std::fs`-style path methods and `File` and `ReadDir`
  handles; it needs `std` in `sync` and `alloc` in `r#async`. `File`
  implements the `hadris-io` traits, and `std::io` in `sync`.
  `copy_tree`, `extract_to_host`, `import_from_host`, `TreeExt` and the
  contract kit work on the new trait; the kit also checks that invalid
  names fail with `InvalidInput`. `Name::new` no longer validates; call
  `Name::check`. `NodeId` wraps a `NonZeroU64`. `Capabilities` is built with
  `Capabilities::new(CaseRule, Charset, max_name_bytes)` and reports which
  fields a format stores (`stores(Field) -> Stored`). `Metadata` has a
  builder, `Permissions` replaces `Mode`, and `SetAttr` describes changes.
- **hadris-fat, hadris-ntfs, hadris-iso, hadris-udf, hadris-block,
  hadris-optical (V3):** `FatFs`, `ExFatFs`, `NtfsFs`, `IsoView`, `UdfFs`,
  `OpenVolume` and `OpenOpticalImage` implement `FileSystem` directly; the
  core methods moved from inherent methods into the trait. `FatFs` and
  `ExFatFs` keep the old label getter as `volume_label`. An ISO view's
  `label` decodes the volume identifier of the descriptor it reads (UCS-2
  for Joliet), and a UDF volume's is its logical volume identifier. UDF
  listings read each entry's file entry, so a damaged entry fails the
  listing.
- **hadris-fat (V3):** `FatFs<D>` and `ExFatFs<D>` have one type
  parameter and need `alloc`. They mount with `mount(dev, MountOptions)`
  and give the device back with `unmount`, which syncs first; `open` and
  `open_with` are removed. The node table is private and unbounded unless
  `MountOptions::with_node_limit` caps it. The clock, code page and UTC
  offset are runtime options, and `clock()` and `code_page()` return
  `&'static dyn` references. Short names default to CP437 instead of
  `Ascii`. `FormatOptions::with_clock` (FAT and exFAT) takes a
  `&'static dyn Clock`, and `FormatOptions` is `Copy` with no type
  parameter. Without `alloc` the crate keeps `check` and the raw layer.
- **hadris-fat (V3):** FAT timestamps are read and written as local time
  in the zone `MountOptions::with_utc_offset` names, and carry that offset.
  exFAT reads times without a valid offset field in that zone and stamps
  new times with it. `hadris_fat_raw::date::{decode, encode}` and
  `exfat::decode_time` take the zone.
- **hadris-block (V3):** `OpenVolume` needs `alloc`, as the FAT drivers do;
  detection does not.
- **hadris-fat (V3):** `sync::check`, `r#async::check` and
  `async_send::check` are the `hadris-fat-raw` checker: they take an
  unmounted device and a scratch buffer instead of a `FatFs`, and report
  `hadris_fs::Finding`s, whose `detail` is a `hadris_fat::Detail`, instead
  of the `Finding` enum. `CheckReport` counts findings and passes; the
  file, directory and cluster counts are gone (use `FatFs::stats`).
  `exfat::sync::check` and its twins change the same way, with
  `hadris_fat::exfat::Detail` codes.
- **hadris-fat-cli:** `verify` prints each finding as `message: path
  (location)` with its severity, and `stat` counts files and directories
  by walking the tree. `stat` no longer prints bad clusters, and `verify`
  no longer prints file, directory, bad and lost cluster counts.
- **hadris-fat (V3):** `hadris_fat::raw` is the `hadris-fat-raw` crate and
  `hadris_fat::exfat::raw` its `exfat` module. The layouts keep their
  names; `FatKind` is re-exported from there.
- **hadris-fat (V3):** `FatFs` and `ExFatFs` mount a device that is not
  `writable` read-only, as if `MountOptions::with_read_only` were set.
- **hadris-part (V3):** `open` returns a `hadris_storage::Partition` of the
  disk instead of a `Slice`.
- **hadris-storage (V3):** `file_len` moves to `host::file_len`.
  `MemBuffer` gains `writable`, true unless overridden and false for
  `&[u8]`, and `StreamWrite` gains `stream_writable`, false for
  `ReadOnly`, so `MemDevice` and `StreamDevice` report `writable`.

- **hadris-iso, hadris-udf, hadris-cpio, hadris-part, hadris-ntfs,
  hadris-cd, hadris-block, hadris-optical (V3):** Each crate returns
  `hadris_fs::Error<E>` and keeps a `Detail` enum of fieldless, numbered
  codes. `Detail::of(&err)` reads the detail of an `Error`,
  `Detail::from_code(code)` that of a `PathError` or any `DetailCode`, and
  `Detail::code()` gives the stable code in the crate's domain (such as
  `hadris-iso`). `hadris-part`'s `Overlap`, `OutOfBounds` and
  `NoSuchPartition` lose their fields; `TableError::index()` and
  `other()` keep the partition numbers.
- **hadris-iso, hadris-udf, hadris-cpio, hadris-cd (V3):** The writers
  (`plan`, `write`, `Session::write`, `CpioWriter`) return `PathError`,
  and an error from a file's content, or from an entry, carries the file's
  path in the tree. `hadris-cd` keeps the detail code of the ISO 9660 or
  UDF writer that failed, readable with that crate's
  `Detail::from_code`.
- **hadris-iso, hadris-udf, hadris-cpio, hadris-part, hadris-ntfs,
  hadris-fat (V3):** Bytes that are not the format at all fail with
  `ErrorKind::NotRecognized` instead of `Corrupt` or `NotFound`: an ISO
  9660 image whose first volume descriptor lacks `CD001`, a UDF volume
  without an NSR descriptor, a cpio archive whose first header has no
  magic, a disk with no MBR or GPT, a device without an NTFS boot sector,
  a FAT boot sector whose sector or cluster size FAT does not allow, and an
  exFAT first sector that does not name exFAT.
- **hadris-block, hadris-optical (V3):** `open` and `open_detected` return
  `hadris_fs::MountError`. A device with no known format fails with
  `ErrorKind::NotRecognized`, and a driver that refuses the volume returns
  its own error, with its own detail code.
- **hadris-fs (V3):** `AnyError` is now `PathError`, the error of writers,
  tree edits, `copy_tree` and code that mixes devices. It keeps the whole
  context of an `Error` (kind, message, location, detail), the path that
  failed within the tree or volume (`with_path`, `path`) and, with `std`,
  the host path (`with_host_path`, `host_path`), and boxes the device or
  source error as its `source()`. `Display` appends the path. Every
  `Error<E>` and `MountError<D, E>` converts with `?`. Reading a host file
  as tree content fails with the host path set. Converting into
  `std::io::Error` returns an `io::Error` source as itself and otherwise
  keeps the `PathError` as the payload.
- **hadris-fs (V3):** The lexical path error of `path::VPath::normalize` is
  `path::NormalizeError`, formerly `path::PathError`.
- **hadris (V3):** `PathError` is re-exported at the crate root with
  `alloc`.
- **hadris-storage (V3):** `BlockDevice::read_blocks`, `write_blocks` and
  `flush` return `hadris_io::Error<Self::Error>`. A device failure is
  `Error::device`, a refused write is kind `ReadOnly`, and a request an
  adapter refuses itself, such as one past the end of a `Slice` or a
  `MemDevice`, is kind `InvalidInput` with the block as its location.
  Adapters keep the device's error type: `Slice<D>` and `StreamDevice<T>`
  report `D::Error` and `T::Error`, `Cache<D>` reports `D::Error`,
  `ByteView<D>` reports `Error<D::Error>` from `read_at`, `write_at` and its
  stream traits, and `MemDevice` reports `Infallible`. A short stream under
  a `StreamDevice` fails with kind `InvalidInput`.
- **hadris-fs (V3):** `Error<E>`, `ErrorKind` and `FsResult` are
  re-exported from `hadris-io`. `Display` shows the message and location;
  a device error is only the `source()`, so chain printers show each text
  once. Converting an error without a device error into `std::io::Error`
  keeps it as the payload, an `Error<Infallible>`, instead of the bare
  `ErrorKind`.
- **hadris-block, hadris-optical (V3):** `detect` returns
  `hadris_fs::FsResult`, since block reads return `hadris_fs::Error`.
- **hadris-storage (V3):** `Cache` keeps its blocks on an LRU list and its
  dirty blocks in an ordered set, so a miss, an eviction and a flush no
  longer scan every slot (a 64 MiB FAT32 copy through a 65536-block cache
  went from 8 to 200 MiB/s). Consecutive missing blocks are read in one
  device call, a flush writes each run of consecutive dirty blocks in one
  call, and requests of at least `capacity` blocks go straight to the
  device. Write-back behaviour is unchanged.
- **hadris-fat (V3):** Directory scans are cheaper. `FatFs` checks the
  first hashed short-name candidate in the same scan as the duplicate
  check and the free-slot search, parses slots in place in its block
  buffer and folds ASCII without Unicode tables. `ExFatFs` lookups and
  creates compare the stored name length and `NameHash` before reading an
  entry set, so an entry set with a wrong `NameHash` (a `check` finding)
  is no longer found by name. Creating 10,000 files in one directory went
  from 18 to 6.5 s on FAT32 and from 24 to 3.7 s on exFAT.
- **hadris-fat (V3):** `read_at` and `write_at` of `FatFs` and `ExFatFs`
  read and write each run of clusters that follow one another on disk in
  one device call instead of one call per cluster. Growing a file by
  several clusters writes the FAT (and on exFAT the allocation bitmap) a
  device block at a time, the chain's end first, and freeing a chain
  clears its entries a block at a time; FAT12 keeps writing entry by
  entry. Writing 64 MiB with 4 KiB clusters now takes 578 device writes
  on FAT32 (was 81,922) and 397 on exFAT (was 65,545).
- **hadris-fat (V3):** `read_dir_entry` resumes from where the last call
  on that directory left its cluster chain instead of walking the chain
  from the start, and finding the node pinned at an entry is a lookup by
  id in the node table (logarithmic in `HeapTable`) until a pinned node
  is renamed or removed, instead of a search of every pinned node. 12,000
  lookups with all of them kept pinned went from 144 to 20 ms.
- **hadris-fat (V3):** `FatFs` and `ExFatFs` use less stack. Mount moves
  its block buffer (and the exFAT up-case table) into the driver once
  instead of through nested `Result`s; long names are encoded per entry
  instead of into copied 520-byte buffers; exFAT entry sets no longer
  carry a decoded name and scans fill a set the caller owns. On a
  Cortex-M4 with 512-byte sectors, peak stack beyond the driver itself
  falls from 10.3 to 9.8 KiB for a FAT32 mount, 4.6 to 1.7 KiB for create
  and append and 5.7 to 3.3 KiB for a replacing rename; for exFAT from
  33.7 to 12.8 KiB for a mount, 11.3 to 4.1 KiB for create and append and
  18.0 to 5.6 KiB for rename. Async futures shrink by up to a half.
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
  The relocation directory is `rr_moved` or `.rr_moved`, chosen with
  `RockRidge::with_relocation(Relocation::RrMoved)` (the default) or
  `Relocation::DotRrMoved`, the only names libarchive and `bsdtar` read
  relocated directories from; a clash at the root fails instead of falling
  back to the other name. Files of 4 GiB or more need `IsoLevel::L3`.
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

- **hadris-cli (V3):** The `hadris-fat-cli`, `hadris-iso-cli`,
  `hadris-udf-cli`, `hadris-cpio-cli` and `hadris-cd-cli` packages and
  their binaries (`hadris-fat`, `fatutil`, `hadris-iso`, `hadris-iso-cli`,
  `hadris-udf`, `hadris-udf-cli`, `hadris-cpio`, `cpioutil`, `hadris-cd`).
  `hadris-cd info` has no successor beyond `hadris detect` and the `info`
  commands of `hadris iso` and `hadris udf`, and the bridge writer's
  defaults now follow `hadris iso create` (level 1, no Joliet unless `-J`).
- **hadris-block, hadris-optical, hadris-cd (V3):** The three crates are
  gone. Detection and opening are `hadris::{sync, r#async}::{detect,
  open, AnyFs}` and `hadris::host::open`: `OpenVolume`,
  `OpenOpticalImage`, `OpenPolicy`, `BlockFormat`, `FatVariant`,
  `OpticalFormats` and their `Detail` types have no successor beyond
  `ImageFormat`, `Detection` and `Candidate`. The bridge writer is
  `hadris_udf::plan_bridge` and `hadris_udf::{sync, r#async}::write_bridge`,
  which take `IsoOptions` and `UdfOptions` in place of `CdOptions`. The
  umbrella drops the `block`, `optical` and `cd` features and the
  `hadris::block`, `hadris::optical` and `hadris::cd` modules; enable
  `detect` and `part` instead.
- **hadris-iso (V3):** `IsoFs::block_size`, `volume_blocks`,
  `boot_catalog_block`, `primary_descriptor` and `raw_record`, the owned
  `BootCatalog` and `BootCatalogEntry` (now `CatalogEntry`). Use `info()`,
  `BootCatalog::block`, `records` with `read_raw`, and
  `boot_catalog(&mut buf)`.
- **hadris-udf (V3):** `UdfFs::volume_id`, `logical_volume_id`, `revision`,
  `block_size`, `partitions` and `read_bytes`, and `Partition` (now
  `PartitionInfo`). Use `info()` and `read_raw`.
- **hadris-fat (V3):** `FatFs::kind`, `FatFs::volume_label`,
  `ExFatFs::volume_label`, `ExFatFs::volume_id`, `ExFatFs::cluster_size`
  and `cluster_chain` on both. Use `info().kind()`, `FileSystem::label`,
  `info().volume_serial()`, `info().cluster_size()` and `extents`.
- **hadris-iso (V3):** `VolumeIdentifiers`, `RockRidge`, `Charset` with
  `with_charset`, `PartitionScheme`, `HybridBoot::with_efi_partition` and
  `JolietLevel` as a writer option. Use `with_id`, `with_rock_ridge`, an
  appended partition, and `with_joliet()`, which writes level 3.
- **hadris-udf (V3):** `UdfOptions::with_volume_id` and `volume_id`; use
  `with_id(UdfId::Volume, ..)` and `id`.

- **hadris-fs (V3):** `FileTimes`, `SetMetadata` and `DeviceKind`, with
  the old `tree` module (`add_file`, `add_dir`, `add_symlink`,
  `add_device`, `add_hard_link`, `set_metadata`, `content_mut`,
  `Tree::from_fs`, `FromFsOptions`, `TreeExt::from_filesystem`,
  `Content::source` and `Content::path`), and `extract_to_host` and
  `import_from_host`. Use `Tree::insert`, `link` and `replace`,
  `host::read_tree`, `host::file`, `read_tree` and `host::write_tree`.
  `Content::source` returns in 3.x.
- **hadris-cpio (V3):** `NewEntry`, the crate's `Report` (with
  `entries` and `size_bytes`), and `CpioWriter::write_tree` and
  `warnings`.
- **hadris-iso, hadris-udf, hadris-cd (V3):** Their own `Report` types
  (`total_blocks`, `size_bytes`, `extent_of`, `allocated_end`), `plan` in
  the mode modules, and `with_clock`. `RockRidgeInfo::times`.
  `hadris_udf::Bridge` and `UdfOptions::with_bridge`, now internal to
  `write_bridge`. `hadris_cd::Detail`, which no error carries any more.
- **hadris-fat (V3):** `hadris_fat::raw` and `hadris_fat::exfat::raw`, which
  re-exported the whole `hadris-fat-raw` crate and tied `hadris-fat`'s API
  to its version. `hadris-fat` keeps `FatKind`, `Detail`, `exfat::Detail`
  and the `check` functions; depend on `hadris-fat-raw` for the layouts,
  codecs and device primitives.
- **hadris-fs (V3):** `NodeTable`, `FixedTable`, `HeapTable` and
  `TableFull`; drivers keep their node tables private.
- **hadris-fat (V3):** `MountOptions` and `exfat::MountOptions`, replaced by
  `hadris_fs::MountOptions`, and `CodePage`, `Ascii` and `Cp437`, which
  moved to `hadris-fs`.
- **hadris-fs (V3):** `FsDriver`, the `&self` `FileSystem`, `AsDriver`,
  `Access`, `OpenFile`, the `File<A>` and `Dir<A>` handles, `DirItem`,
  `DriverExt`, `PathExt`, the `Lexical`, `Posix`, `Resolver` and
  `WithResolver` resolvers, the lock kinds (`LockKind`, `Lock`,
  `StdMutex`, `Spin`, `AsyncMutex`), `impl_fs_driver!`, the `path` module,
  `NewNode`, `RemoveKind`, `RenameFlags`, `Mode`, `CaseSensitivity` and
  `NameCharset`. Their jobs moved to `FileSystem`, `Volume` and the
  vocabulary above.
- **hadris-fat (V3):** `check_with`, the `Finding` enums, `FindingKind` and
  the `CheckReport` types of FAT and exFAT; use `check` and
  `hadris_fs::Finding`. The `defmt` feature no longer derives on findings.
- **hadris-storage (V3):** `impl BlockDevice for std::fs::File`, since
  `block_count` cannot fail and a `File` cannot tell how it was opened;
  use `host::FileDevice`. `Slice` is replaced by `Partition`.
- **hadris-io (V3):** `impl ErrorType for std::fs::File`, which only the
  removed `File` device used.

- **hadris-iso, hadris-udf, hadris-cpio, hadris-part, hadris-ntfs,
  hadris-cd, hadris-block, hadris-optical (V3):** The per-crate `Error<E>`
  wrappers and their `content_error`; use `hadris_fs::Error`,
  `hadris_fs::PathError` and `Detail::of`. `hadris-block` and
  `hadris-optical` lose `OpenError` (use `MountError`) and the
  `UnknownFormat` and `Mount` details; `hadris-cd` loses `Detail::Iso` and
  `Detail::Udf`; `hadris-cpio` loses `Detail::Content`.
- **hadris-storage (V3):** `WriteError`, `StorageError` and `OutOfRange`.
  Every block operation returns `hadris_io::Error`; see Changed.
- **hadris-fs (V3):** `Error::from_device`; use `Error::device(err,
  message)`.
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

- **hadris-storage (V3):** A `std::fs::File` opened on a disk device
  reports its size on every platform, through the new
  `hadris_storage::host::file_len`: the `DKIOCGETBLOCKCOUNT` and
  `DKIOCGETBLOCKSIZE` ioctls on macOS, `DIOCGMEDIASIZE` on FreeBSD,
  `IOCTL_DISK_GET_LENGTH_INFO` on Windows (`\\.\PhysicalDriveN` and volume
  paths) and a seek to the end elsewhere. `stat` and `lseek` give 0 for such
  devices, so they had no blocks and every image on them failed to open.
  `file_len` fails with `ErrorKind::Unsupported` instead of returning 0 for
  a device it cannot measure. The `hadris-iso verify` command measures its
  input the same way instead of from file metadata.
- **hadris-fs (V3):** A disk device given as `Content::path` is read in
  full. Its length came from file metadata, so it was stored as an empty
  file.
- **hadris-fs (V3):** `TreeExt::from_filesystem`, `copy_tree` and
  `extract_to_host` fail with `ErrorKind::Corrupt` when a directory entry
  leads back to a directory on its own path, as a corrupt ISO 9660 or UDF
  image can hold, and with `ErrorKind::LimitExceeded` below 1024
  directories. They walked such a tree without end. A copy of a directory
  into itself on one volume now stops at the same depth instead of filling
  the volume.
- **hadris-fat (V3):** exFAT `remove`, and `rename` when it replaces a
  node, free the clusters the node's Vendor Allocation entries (and other
  benign secondary entries with an allocation) hold. They were left
  allocated, and `check` reported them as lost.
- **hadris-iso (V3):** A tree that needs Rock Ridge relocation is refused
  with `Detail::Relocation` when its root holds a directory named `rr_moved`
  or `.rr_moved`, other than the relocation directory, whose record comes
  first: `.rr_moved` with `NameCase::Preserve` and the default `rr_moved`,
  or `rr_moved` with a `.rr_moved` relocation directory. libarchive takes
  that directory for the relocation directory, and `bsdtar` failed on the
  image with "Invalid Rockridge RE".
- **hadris-iso (V3):** A session that keeps the boot catalog and replaces
  a no-emulation boot image sets the entry's load size from the new image
  when the entry loaded the old one whole, and writes a boot information
  table into the new image when the old one held one. The entry kept the
  old load size, and the table was not written, so firmware loaded part of
  the new image and loaders that check the table, such as ISOLINUX, failed.
- **hadris-cd (V3):** The volume space size of the ISO 9660 volume
  descriptors of a bridge image covers the whole image. It left out the
  UDF structures and trailing anchor after the ISO 9660 image, so readers
  that trust it saw a volume shorter than the image.
- **hadris-iso (V3):** Primary and enhanced tree identifiers map each
  character of a name to one byte, so `café.txt` becomes `CAF_.TXT;1`
  instead of `CAF__.TXT;1`, and Level 1 and 2 limits count characters.
  A Level 2 or enhanced name that is too long loses the end of its base
  name and keeps its extension, cut only when the extension alone does not
  fit; before, a base name of 30 characters or more dropped the extension.
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
- **hadris-fat (V3):** `FatFs` and `ExFatFs` fail with `Corrupt` when a
  directory's or file's cluster chain loops, instead of listing the
  directory's entries or the file's clusters over and over. Detection uses
  Brent's algorithm on every chain walk, including walks that resume from
  a file's remembered position, so it needs no memory per cluster.
- **hadris-fat (V3):** The default `Ascii` code page reads a short-name
  byte `b` above `0x7F` as the private-use character `U+F700 + b` instead
  of U+FFFD, and encodes those characters back to their bytes. Short names
  without a long name that differed only in such bytes listed as one name,
  and a lookup of it found only the first.
- **hadris-fat (V3):** A `FatFs` operation interrupted by a dropped
  `async` future or a failed write no longer leaks clusters for good or
  leaves the FAT copies different after `sync`. The driver remembers the
  clusters an unfinished operation allocated but had not linked, or had
  unlinked but not freed, the long-name entries it may have left without
  their short entry, and a FAT entry it had not yet mirrored; the next
  `create`, `remove`, `rename`, `write_at`, `set_len`, `set_metadata` or
  `sync` frees, clears or mirrors them, unless the interrupted write did
  land. A growing write that was dropped has its chain cut back to the
  file's size. The free count follows each change of the active FAT, so
  it stays exact across interruptions.
- **hadris-fat (V3):** `ExFatFs` entry sets survive interrupted
  operations. The entries of a set that lie in one device block are
  written or cleared with one device write, so a set within one block is
  never half written; across a block boundary new sets are written File
  entry last and removed sets lose it first. The driver remembers the set
  it was writing and the clusters an unfinished operation held, and the
  next writing operation or `sync` removes a new set that did not land
  whole, completes a removal, reseals an updated set (checksum and name
  hash), frees clusters that were never linked or were unlinked but not
  freed, and cuts back a chain a dropped write had grown. A dropped
  `create`, `write_at` or `remove` left orphaned secondary entries or a
  bad `SetChecksum`, after which `sync` failed with `Corrupt` and
  `fsck.exfat` rejected the volume.
- **hadris-fat (V3):** `sync` of `FatFs` and `ExFatFs` writes every other
  pending node, and flushes the device, when one node's entry can no
  longer be read; it then fails with `Corrupt` once and drops that node's
  pending size, instead of failing before the rest on every call.
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
- **hadris-iso (V3):** Level 1 and 2 file identifiers without an extension
  keep the separator (`README.;1`), as ECMA-119 7.5.1 requires, and names
  split at their last dot (`x.tar.gz` becomes `X_TAR.GZ`). Joliet names
  replace the characters Joliet forbids (`* / : ; ? \` and controls) with
  `_`, and the `Report` warns when a Joliet name is cut to 64 characters,
  loses characters outside the BMP or forbidden ones, or differs from a
  sibling only in case.
- **hadris-iso (V3):** Every directory record of a file larger than 4 GiB
  carries its Rock Ridge entries (`PX`, `TF`, `NM`), as libisofs writes
  them. Only the first did, so `bsdtar` refused the image and xorriso
  showed the file with mode `0000` under its primary name.
- **hadris-iso (V3):** Images and sessions end in 150 zero blocks counted
  in the volume space size, as xorriso and `mkisofs -pad` write by
  default. `isoinfo` refused images shorter than 48 blocks ("Short read on
  old image").
- **hadris-iso (V3):** `HybridBoot::with_bootstrap` code over 446 bytes
  fails with `ErrorKind::LimitExceeded` and `Detail::HybridBoot` instead of
  being cut. El Torito entries are checked before writing: a load size of
  zero and a diskette image that is not exactly its diskette's size fail
  with `Detail::BootImage`, as `mkisofs` refuses them; a no-emulation load
  size past the image is written and reported as a warning. A GPT or
  hybrid table with several UEFI entries and no
  `HybridBoot::with_efi_partition` warns that it has no EFI system
  partition.
- **hadris-iso, hadris-udf (V3):** A damaged record or entry reached
  through a node id the driver listed fails with `ErrorKind::Corrupt`
  instead of `InvalidHandle`. `InvalidHandle` is kept for ids no record can
  have: for ISO zero, odd or past the volume and the device, for UDF
  outside every partition. A directory record or file identifier that
  points outside the image fails the listing as corrupt.
- **hadris-udf (V3):** The writer refuses UDF 2.50 and 2.60 with
  `ErrorKind::Unsupported` and `Detail::PartitionMap`: those revisions
  require a metadata partition (UDF 2.50 2.2.10), which it does not write.
  It labelled type 1 volumes 2.50 or 2.60 before, which conforming readers
  may refuse.
- **hadris-iso (V3):** `Session` keeps its boot catalog in step with the
  tree: an entry whose boot image was replaced points at the new content,
  patched in the kept catalog, and removing a boot image the catalog loads
  fails with `Detail::BootImage` instead of leaving the old loader
  bootable. Sessions start after every partition and backup GPT, so
  partitions `xorriso -append_partition` put after the ISO data are no
  longer overwritten by `Append` or refused by `Rewrite` with a misleading
  `Detail::HybridBoot`; a partition that cannot grow without overlapping
  another keeps its size and is reported. `Append` warns when the options
  have hybrid boot it does not apply, and the `Session::write` and
  `SessionMode` docs say which mode writes the system area.
- **hadris-iso (V3):** In the Rock Ridge view, the names of a hard link
  share one node id: the first record in path table order with the same
  `PX` serial number, or without one, the same data extent. They had one
  id per name while reporting two links, so `Tree::from_filesystem` and
  FUSE adapters split them into separate files.
- **hadris-iso (V3):** When deep directories need Rock Ridge relocation and
  the root already holds a directory with the relocation name (`rr_moved`
  by default), that directory is reused as the container and keeps its own
  entries, instead of failing with `Detail::Relocation`. It is laid out
  first, as for a new container, so libarchive/bsdtar can extract it. A
  root file with that name still fails.
- **hadris-cpio-cli:** `extract` replaces a symlink where a directory entry
  lands instead of keeping it, so an archive holding `link -> /outside` and
  then a directory `link` no longer changes the mode of `/outside`.

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

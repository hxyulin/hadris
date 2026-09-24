# Hadris V3 action catalog

Requirements only. This file lists what a user can do with each format and what Hadris should support. It proposes no API; the API is designed against it. Current support is tracked by the conformance tests, not here.

Each action has a stable ID. A conformance test in hadris-tests covers each ID. If an action marked `3.0` does not work, that is a bug.

## 0. How to read the tables

**Columns**

- **ID**: `<AREA>-<VERB>-NN`. An ID shared by several formats means the same action; the format columns give the differences.
- **Class**:
  - `core` is one of the closed set of operations every mounted filesystem has (section 2).
  - `builder` is image construction or remastering.
  - `extra` is a format-specific or tool-level action.
- **Format columns F, X, I, U, C**: F = FAT12/16/32 with VFAT long names, X = exFAT, I = ISO 9660 with its extensions, U = UDF, C = cpio. Each cell says whether the format can do the action at all:
  - `Y` = yes, the format supports it.
  - `N` = the format cannot express it.
  - `-` = not applicable (for example a mounted-write action on a format that is only built and read).
  - `B` = the format supports it only when building the image, not on a mounted volume.
  - A short note follows where the semantics differ.
- **Users**: `emb` = embedded firmware (no alloc, tiny stack), `ker` = OS kernel or VFS/FUSE, `host` = host tools and servers, `bld` = image builders (osdev boot media, initramfs), `insp` = inspection and forensics.
- **Want**: `3.0`, `3.x` (later, additive), or `no`, followed by a short reason.

---

## 1. Actions

### 1.1 IO: devices, offsets, block sizes, detection

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| IO-OPEN-01 | Open a filesystem on a block device whose logical block size is 512, 1024, 2048 or 4096 bytes, and reject larger sizes with Unsupported, not corruption. ISO needs blocks of at most 2048 bytes and refuses a 4096-byte device with Unsupported | core | Y 512-4096 | Y 512-4096 | Y 2048 normal; 512-2048 legal | Y 512-4096 | - byte stream | all | 3.0: 4Kn disks and optical media are common |
| IO-PART-01 | Open a filesystem inside a partition given as a byte or block window of a larger device (MBR, GPT, or a hybrid ISO's partition) | core | Y | Y | Y | Y | - | all | 3.0: USB sticks, disk images and ESPs are always partitioned |
| IO-RO-01 | Open on a read-only device (no Write) and get a read-only filesystem. Opening a file for writing fails at open time with ReadOnly, before any truncation | core | Y | Y | Y | Y | Y | all | 3.0: read-only media, forensics write blockers |
| IO-RO-02 | A write refused by the device mid-operation leaves the volume consistent and turns the mount read-only | core | Y | Y | - | - | - | emb, ker | 3.0: SD cards lock themselves; the volume refuses further writes until remount, like errors=remount-ro (decided 2026-09-24) |
| IO-STREAM-01 | Read an archive from a non-seekable stream (pipe, stdin, decompressor output) | builder | - | - | N (needs seek) | N | Y | host, bld, insp | 3.0 for cpio: `zcat initrd \| tool` is the main use |
| IO-STREAM-02 | Write an image to a non-seekable sink (stdout, compressor, network) | builder | N | N | Y two-pass | Y standalone | Y | bld, host | 3.0 for cpio; ISO and UDF 3.x: design 5.2 plans `write_stream` |
| IO-GROW-01 | Write an image to a growable target (in-memory `Vec`, host file) with no presizing | builder | Y (format needs a size) | Y (same) | Y | Y | Y | bld, host | 3.0: integrate-28, every test and in-memory builder needs it |
| IO-SPARSE-01 | Leave long zero runs as holes when writing an image to a host file | builder | Y | Y | Y | Y | N | bld, host | 3.x: disk use only; output bytes do not change |
| IO-DETECT-01 | Detect the format of an unknown device: FAT12/16/32, exFAT, ISO, UDF, ISO+UDF bridge, cpio (each variant), partition table, NTFS. Tell "not this format" apart from "this format but damaged" | extra | Y | Y | Y | Y | Y (magic) | insp, host, ker | 3.0: open-anything tools and CLI errors depend on it (dx-10, inspect-6) |
| IO-DETECT-02 | Open whatever the detector found as one generic read-only filesystem and list it | extra | Y | Y | Y | Y | Y (needs index) | insp, host | 3.0 for F/X/I/U (decision S2, `hadris::open`); cpio 3.x (read-only cpio driver deferred, decided 2026-09-24) |
| IO-DETECT-03 | Detect compressed input (gzip, zstd, xz, lz4) and name it in the error, without decompressing | extra | - | - | - | - | Y | host, insp | 3.x: better error text only; decompression stays out of scope |
| IO-TRUNC-01 | Open a truncated image leniently: read what is present, report "truncated" as its own error kind, not a raw I/O error | extra | Y | Y | Y | Y | Y | insp | 3.x: forensics; strict open stays the default (inspect-16) |
| IO-INTO-01 | Get the device back after unmount or after a failed mount, so the caller can try another format | core | Y | Y | Y | Y | Y | all | 3.0: fallback probing (dx UC3) |
| IO-HOST-01 | Open an image or a host block device (`/dev/sdX`, `\\.\PhysicalDrive`) by path, sized correctly (seek to end, not metadata length) | extra | Y | Y | Y | Y | Y | host, insp | 3.0: decision 4.15 host module |
| IO-RAW2352-01 | Read CD images with 2352-byte raw sectors (bin/cue, Mode 1 and Mode 2 Form 1) | extra | - | - | Y | Y | - | insp | no: a user-written block device adapter covers it; convert to .iso first |
| IO-TRIM-01 | Discard freed clusters on the device (TRIM or erase) after delete or truncate | extra | Y | Y | - | Y (rw) | - | emb, host | 3.x: flash wear and SSD images; needs an optional device capability |

### 1.2 VOL: format, mount, label, serial, space, info

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| VOL-FORMAT-01 | Format a whole device with automatic choice of FAT type and cluster size by size (FAT12 below 16 MiB, FAT16 below 512 MiB, FAT32 above; Microsoft tables) | extra | Y | Y | - | Y (mkudffs) | - | emb, bld, host | 3.0: mkfs is the entry point for every FAT user |
| VOL-FORMAT-02 | Format with explicit parameters (type, cluster size, sector size, reserved sectors, FAT count, root entries, media byte, OEM name, label, serial) and reject impossible combinations before writing anything | extra | Y | Y (no root entries or media byte; adds alignment, FAT count 1 or 2) | - | - | - | bld, emb | 3.0: osdev images and SD card conformance need fixed layouts |
| VOL-FORMAT-03 | Format inside a partition and record the partition start: FAT hidden sectors, exFAT PartitionOffset, taken from the partition window | extra | Y | Y | - | - | - | bld | 3.0: Windows and some firmware refuse ESPs with hidden sectors = 0 (osdev-11) |
| VOL-FORMAT-04 | Format with legacy CHS geometry and floppy presets (360K to 2.88M: sectors per track, heads, media byte, root entries) | extra | Y | N | - | - | - | bld | 3.x: floppy boot images (osdev-12); additive option |
| VOL-FORMAT-05 | Format with the data area and clusters aligned to a given boundary (flash erase block, SD Association layout) | extra | Y | Y | - | - | - | emb, bld | 3.0 for FAT and exFAT: one format option, SD cards are the main embedded target (decided 2026-09-24) |
| VOL-FORMAT-06 | Format an empty UDF filesystem (Type 1 partition) on a rewritable device for later read-write use, as mkudffs does | extra | - | - | - | Y | - | host | 3.x: only useful once UDF write mount (VOL-MOUNT-05) exists |
| VOL-FORMAT-07 | Format and get back a mounted volume with caller-chosen mount options (clock, node limit, UTC offset, code page), not a fixed default | extra | Y | Y | - | - | - | emb, host, ker | 3.0: server-13, embedded-9, integrate-18 |
| VOL-FORMAT-08 | Full format: zero or discard the data area, or scan for bad blocks (mkfs.fat -c) | extra | Y | Y | - | Y | - | host | no: device-level job, and slow; the user zeroes the device |
| VOL-MOUNT-01 | Mount read-write with default options and a caller clock | core | Y | Y | - | Y (Type 1) | - | all | 3.0 for F/X; UDF see VOL-MOUNT-05 |
| VOL-MOUNT-02 | Mount read-only on purpose: no byte written, including dirty flags and FSInfo | core | Y | Y | Y | Y | Y | insp, ker | 3.0: forensic integrity |
| VOL-MOUNT-03 | Mount options: clock, UTC offset for FAT local times, OEM code page, cap on open nodes | extra | Y | Y (UTC offset stored per entry) | - | - | - | all | 3.0: decisions D7, D10 |
| VOL-MOUNT-04 | Handle the clean-shutdown state: report a volume that was left dirty, set the flag on the first write, clear it on clean sync or unmount (FAT16/32 FAT[1] bits, exFAT VolumeDirty) | extra | Y (FAT16/32 only) | Y | - | Y (LVID open/closed) | - | ker, emb, insp | 3.0: set the flag on the first write, not on mount (decided 2026-09-24); clear on clean sync or unmount |
| VOL-MOUNT-05 | Mount UDF read-write on random-access media (Type 1 partition): create, write, remove, rename, then close the LVID | core | - | - | - | Y | - | host, ker | 3.x: must be addable without changing any 3.0 signature (NF-STABLE-02) |
| VOL-MOUNT-06 | Mount a volume with a damaged primary boot sector from the backup (FAT32 sector 6, exFAT backup boot region), read-only | extra | Y (FAT32) | Y | - | Y (reserve VDS, other anchors) | - | insp, ker | 3.0 for F, X and U, read-only (decided 2026-09-24) |
| VOL-UMOUNT-01 | Unmount: sync all pending metadata, report errors, return the device | core | Y | Y | Y | Y | Y | all | 3.0: close returns Result, not a silent Drop |
| VOL-SYNC-01 | Sync the volume: dirty FAT sectors to every FAT copy, directory entries, FSInfo, exFAT PercentInUse and bitmap, then flush the device | core | Y | Y | - | Y (rw) | - | all | 3.0: durability contract |
| VOL-LABEL-01 | Read the volume label as text. FAT: the root label entry, falling back to the BPB. exFAT: UTF-16 label entry. ISO: PVD volume id, or the Joliet SVD id when a Joliet SVD exists and its id is not empty. UDF: logical volume id | core | Y | Y | Y | Y | N | all | 3.0: decision D2 (`label()` everywhere, defaulted `volume_label()` on the trait) |
| VOL-LABEL-02 | Set or remove the label. FAT writes both the root entry and the BPB and creates the root entry when missing, validating characters. exFAT: 11 UTF-16 units, may grow the root | extra | Y | Y | B | B (Y with rw) | - | host, bld | 3.0 for F/X (fatlabel, exfatlabel); ISO/UDF only at build time |
| VOL-SERIAL-01 | Read the numeric volume serial (FAT BPB volume id, exFAT VolumeSerialNumber) | extra | Y | Y | N | Y (volume set id prefix) | - | insp, ker, host | 3.0: decision D2 (`volume_serial()`); blkid UUID |
| VOL-SERIAL-02 | Change the serial of an existing volume (fatlabel -i, tune.exfat -I) | extra | Y | Y | - | Y | - | host, bld | 3.0: decided 2026-09-24; cheap, fatlabel -i and tune.exfat -I parity |
| VOL-SERIAL-03 | Build UDF images with a unique volume set identifier, derived from the clock or a caller seed, reproducible under a fixed clock | builder | - | - | - | Y | - | bld | 3.0: every Hadris UDF image has the same blkid UUID today (dx-23) |
| VOL-STAT-01 | statfs: total, free and used blocks and the block size. FAT12/16 count free by scanning; FAT32 checks the FSInfo hint before trusting it; UDF uses the LVID free space table | core | Y | Y | Y (free = 0) | Y | N | all | 3.0 |
| VOL-STAT-02 | statfs extras: blocks available to the caller, free file slots (FAT12/16 fixed root entries), file count | core | Y | Y | Y | Y | - | ker | 3.x: FUSE bavail and ffree; design 4.14 |
| VOL-INFO-01 | Read the volume geometry: FAT type, sector size, cluster size, reserved sectors, FAT count and size, root location, cluster count, active FAT; ISO block size and volume space size; UDF revision, block size, partitions | extra | Y | Y | Y | Y | - | insp, host, bld | 3.0: server-14, CLI `info` needs it |
| VOL-INFO-02 | Read the descriptive identifiers and dates: ISO system, volume set, publisher, preparer, application ids, copyright/abstract/bibliographic file ids, creation/modification/expiration/effective dates; UDF implementation and domain ids, recording times; FAT OEM name | extra | Y (OEM) | N | Y | Y | - | insp | 3.0 for ISO and UDF (decided 2026-09-24); FAT OEM name 3.0 |
| VOL-CAPS-01 | Query capabilities before acting: writable, symlinks, hard links, case sensitivity, maximum name length in UTF-8 bytes (FUSE namemax) and charset, which metadata fields are stored, time resolution per field | core | Y | Y | Y | Y | Y | ker, host | 3.0, including which metadata fields and timestamps are stored (needed by the META-TIME-02 rule); per-field time resolution 3.x (4.14) |
| VOL-RESIZE-01 | Grow a FAT or exFAT filesystem in place after its partition grew (extend the FAT, bitmap and cluster count) | extra | Y | Y | N | N | - | bld, host | no: large work, one request (decided 2026-09-24) |
| VOL-RESIZE-02 | Shrink a FAT or exFAT filesystem | extra | Y | Y | N | N | - | host | no: needs relocation, risky, rare |
| VOL-CONVERT-01 | Convert FAT16 to FAT32 in place | extra | Y | - | - | - | - | host | no: niche and risky; reformat plus copy covers it |
| VOL-FAT-01 | Honour FAT32 mirroring flags: use the active FAT when mirroring is off, and keep every copy equal when it is on | extra | Y | Y (TexFAT ActiveFat) | - | - | - | ker, insp | 3.0: correctness on volumes from other systems |
| VOL-TEXFAT-01 | Mount and write TexFAT volumes (two FATs and bitmaps) keeping both copies equal; format with two FATs | extra | - | Y | - | - | - | emb | 3.0 mount and write where done; transactions `no` (Windows CE only) |

### 1.3 DIR: directories

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| DIR-LOOKUP-01 | Look up a name in a directory using the format's rules: FAT and exFAT case-insensitive and case-preserving (exFAT through the volume's up-case table and name hash); ISO primary matches without `;1` and without case; RR, Joliet, UDF and cpio match exactly. An ISO mount uses RR, then Joliet, then the primary names, unless the mount options choose | core | Y | Y | Y | Y | Y (path in archive) | all | 3.0; cpio 3.x (random access needs the 3.x read-only driver, decision 4) |
| DIR-LOOKUP-02 | Look up by the 8.3 alias of a long-named file (`PROGRA~1`) | core | Y | N | - | - | - | ker, host | 3.0: Windows and Linux both allow it; old configs use it |
| DIR-LOOKUP-03 | Look up an ISO name with several versions (`A.TXT;1`, `A.TXT;2`) and get the highest; an explicit `;N` selects that version; a listing shows only the highest version, without `;N`; look up `README` against the stored `README.;1` | core | - | - | Y | - | - | insp, ker | 3.0: ECMA-119 7.5 |
| DIR-LOOKUP-04 | Lookup that also returns the name as stored on disk (case canonicalization) | core | Y | Y | Y | - | - | ker, host | 3.x: design 4.14 |
| DIR-LOOKUP-05 | Match cpio paths with and without a leading `/` or `./` (GNU cpio writes `./x`) | extra | - | - | - | - | Y | host, insp | 3.0: dx-110 |
| DIR-LIST-01 | List a directory with name, type and NodeId per entry, no allocation, through a `Copy` cursor that can be stored and resumed (FUSE readdir offset). `.` and `..` are never returned | core | Y | Y | Y | Y | Y (derived from paths) | all | 3.0; cpio 3.x (random access needs the 3.x read-only driver, decision 4) |
| DIR-LIST-02 | List with the metadata already stored in the directory entry, with no lookup per entry (readdirplus, `ls -l`) | core | Y | Y | Y | Y (needs FE read) | Y | ker, host, insp | 3.0: decision D9; 5000-entry `metadata(path)` took 5.96 s (inspect-4) |
| DIR-LIST-05 | List a directory while it changes: no entry is returned twice, every entry present for the whole listing is returned, and a stored cursor stays valid across creates and removes (POSIX readdir) | core | Y | Y | Y | Y | - | ker | 3.0: FUSE readdir with offsets (decided 2026-09-24) |
| DIR-LIST-03 | List entries the format marks hidden or associated (ISO existence flag, ISO associated files, UDF hidden FIDs) on request, hidden by default only where the OS does so | extra | Y (hidden attr) | Y | Y | Y | - | insp | 3.x: forensics; default behaviour decided in 3.0 |
| DIR-LIST-04 | List deleted entries (FAT 0xE5, exFAT InUse=0, UDF deleted FIDs) for recovery | extra | Y | Y | - | Y | - | insp | no: carving and undelete belong to forensic tools built on the raw crates |
| DIR-MKDIR-01 | Create a directory. FAT allocates a cluster and writes `.` and `..`. A clash with an existing long name, short alias or case variant gives AlreadyExists | core | Y | Y | B | Y (rw) B | B | all | 3.0 |
| DIR-MKDIR-02 | Create every missing parent in a path (create_dir_all), and create exactly one directory, getting AlreadyExists when it exists (create_dir) | core | Y | Y | - | Y (rw) | - | host, emb | 3.0: dx-17 (no single `create_dir` today) |
| DIR-GROW-01 | Grow a directory across clusters until the format limit: FAT 65536 entries, exFAT 256 MiB; the fixed FAT12/16 root gives NoSpace when full | core | Y | Y | - | Y | - | all | 3.0 |
| DIR-RMDIR-01 | Remove an empty directory; a non-empty one gives DirectoryNotEmpty; the root cannot be removed | core | Y | Y | B | Y (rw) | - | all | 3.0 |
| DIR-RMDIR-02 | Remove a tree recursively (remove_dir_all) without heap allocation per level, with a bounded depth that gives LimitExceeded | core | Y | Y | - | Y (rw) | - | emb, host | 3.0 |
| DIR-PARENT-01 | Get a directory's parent (`..`) for POSIX path resolution, including relocated RR directories (PL) and libisofs Joliet `..` records that point at themselves | core | Y | Y | Y | Y | Y | ker | 3.0 |
| DIR-WALK-01 | Walk a tree recursively, node by node, with cycle detection so a corrupt image with a directory loop ends with Corrupt instead of looping | extra | Y | Y | Y | Y | Y | insp, host | 3.0: decision D9; inspect-3 (a cyclic FAT32 chain listed entries about 4096 times), inspect-18 |
| DIR-COMPACT-01 | Compact a directory (drop deleted slots) or sort entries on disk (fatsort) | extra | Y | Y | - | - | - | emb | no: fatsort-style tools only; some MP3 players need it, too niche |

### 1.4 FILE: file content and namespace changes

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| FILE-OPEN-01 | Open by path with read, write, append, truncate, create and create_new. On a read-only volume a write open fails before truncating anything. Opening a directory or a symlink as a file fails clearly | core | Y | Y | Y (read) | Y | Y (read) | all | 3.0; cpio 3.x (random access needs the 3.x read-only driver, decision 4) |
| FILE-OPEN-02 | Open a node by its NodeId with no path (FUSE and kernel open after lookup); path-based open is a layer over lookup plus this | core | Y | Y | Y (read) | Y | - | ker, emb, host | 3.0: FUSE and VFS only have the inode (decided 2026-09-24) |
| FILE-READ-01 | Read at any offset; short read at end of file; a read never changes access times | core | Y | Y | Y | Y | Y (sequential) | all | 3.0 |
| FILE-READ-02 | Read a file larger than 4 GiB: exFAT 64-bit length, ISO multi-extent records (RR metadata on every record), UDF 64-bit; odc up to 8 GiB | core | N (4 GiB - 1 max) | Y | Y | Y | Y odc only; newc and bin 4 GiB - 1 | host, insp, bld | 3.0 |
| FILE-READ-03 | Read data the format marks unwritten and get zeros: exFAT bytes past ValidDataLength, UDF unrecorded or unallocated extents, RR SF sparse files | core | - | Y | Y (SF) | Y | - | all | 3.0 for X and U; RR SF 3.x |
| FILE-READ-04 | Read file data embedded in the UDF ICB (small files) and files described by long and extended allocation descriptors with continuation extents | core | - | - | - | Y | - | all | 3.0 |
| FILE-READ-05 | Read an exFAT NoFatChain (contiguous) file written by Windows or macOS | core | - | Y | - | - | - | all | 3.0 |
| FILE-READ-06 | Read a zisofs-compressed file (RR ZF) transparently | core | - | - | Y | - | - | insp, host | 3.x: design 5.2 feature work; rare outside some live CDs |
| FILE-READ-07 | Read ISO interleaved files (file unit size, gap) and files that span volumes of a multi-volume set | extra | - | - | Y | - | - | insp | no: obsolete. They must fail with a clear Unsupported, never return garbage |
| FILE-READ-08 | Read CD-ROM XA Mode 2 Form 2 files (Video CD) | extra | - | - | Y | - | - | insp | no: needs 2336/2352-byte sectors (IO-RAW2352-01) |
| FILE-WRITE-01 | Write at an offset inside a file, overwriting data in place | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-WRITE-02 | Write past the end of the file: the gap reads back as zeros (FAT writes zeros; exFAT may leave the gap past ValidDataLength) | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-WRITE-03 | Append mode: every write goes to the current end, even with two handles open on the same node | core | Y | Y | - | Y (rw) | Y (stream append) | emb, host | 3.0: data loggers |
| FILE-WRITE-04 | Grow a FAT file to exactly 4 GiB - 1 bytes; one more byte gives FileTooLarge and changes nothing | core | Y | - | - | - | Y (newc limit) | emb, host | 3.0: edge of the format |
| FILE-TRUNC-01 | Shrink a file with set_len and free the clusters (including the whole chain when shrinking to 0; the first cluster field is cleared) | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-TRUNC-02 | Grow a file with set_len: new bytes read as zeros. FAT writes zeros; exFAT raises DataLength without writing | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-ALLOC-01 | Preallocate space without changing the size and without writing zeros (fallocate KEEP_SIZE; exFAT allocation past VDL) | extra | Y (clusters past size; fsck sees them) | Y | - | Y | - | emb, host | 3.x: loggers and video recorders; additive |
| FILE-ALLOC-02 | Create a file in one contiguous run of clusters or fail; exFAT marks it NoFatChain | extra | Y | Y | Y (always) | Y | - | bld, emb | 3.x: stage2 loaders and swap files need it (osdev), additive |
| FILE-EXTENT-01 | Map a file to its device extents (FIEMAP): cluster runs converted to device LBAs; ISO and UDF extents; offset of a cpio entry's data | extra | Y | Y | Y | Y | Y | bld, insp | 3.0: bootloader patching and forensics; cluster chains exist today |
| FILE-CREATE-01 | Create an empty file. Invalid names are refused with a detail naming the bad character. Short alias generated (`~1` to `~4`, then hash tails) with the NT lowercase flags where possible | core | Y | Y | B | Y (rw) B | B | all | 3.0 |
| FILE-CREATE-02 | Create a file exclusively (create_new): AlreadyExists if any name matches, including case-only and short-alias matches | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-UNLINK-01 | Remove a file and free its clusters; unlink of a directory gives IsADirectory; exFAT also frees clusters owned by vendor allocation entries | core | Y | Y | B | Y (rw) | - | all | 3.0 |
| FILE-UNLINK-02 | Remove the last name of a file that is still open: the name goes away, open handles keep reading and writing, space is freed on the last close (POSIX orphan) | core | Y | Y | - | Y (rw) | - | ker | 3.x: design Q3; FUSE `rm` of an open file and atomic saves fail today with Busy (integrate-15) |
| FILE-RENAME-01 | Rename inside a directory, including a case-only rename (`readme` to `README`) on a case-insensitive volume and a rename that changes the number of LFN slots | core | Y | Y | B | Y (rw) | - | all | 3.0 |
| FILE-RENAME-02 | Rename across directories keeping the NodeId | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-RENAME-03 | Rename over an existing file, replacing it (file over file, directory over empty directory) as atomically as the format allows | core | Y | Y | - | Y (rw) | - | all | 3.0: atomic save |
| FILE-RENAME-04 | Move a directory to another parent and rewrite its `..` entry; moving a directory into its own subtree gives InvalidInput | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-RENAME-05 | Rename with NoReplace: AlreadyExists if the target exists | core | Y | Y | - | Y (rw) | - | ker | 3.0 |
| FILE-RENAME-06 | Atomically swap two names (RENAME_EXCHANGE) | core | Y | Y | - | Y (rw) | - | ker | 3.x: design 4.14 |
| FILE-SYNC-01 | Make one file durable (fsync): data, size, times and the FAT chain on disk, then a device flush | core | Y | Y | - | Y (rw) | - | all | 3.0 |
| FILE-CLOSE-01 | Close a handle with a Result, publishing size and times; dropping a written handle publishes its size best effort, so a power cut keeps the data (sync: in Drop; async: Drop cannot await, so the next call on the volume publishes it) | core | Y | Y | Y | Y | - | emb, all | 3.0: embedded-8 (2205 of 2250 bytes lost) |
| FILE-SEEK-01 | Seek from start, end or current position, including past the end; the next write fills the gap | core | Y | Y | Y | Y | N (stream) | all | 3.0 |
| FILE-SHARE-01 | Several handles on one node: each sees the others' writes and size changes at once, and a handle stays valid and keeps working after its file is renamed or moved | core | Y | Y | Y (read) | Y | - | ker, host | 3.0: kernels and servers open one file many times (decided 2026-09-24) |
| FILE-COPY-01 | Copy one file inside the same volume (std::fs::copy) | core | Y | Y | - | Y (rw) | - | host | 3.x: convenience over read and write (dx-17) |
| FILE-HOLE-01 | Report holes (SEEK_HOLE and SEEK_DATA) for UDF unallocated extents and RR SF files | extra | N | N | Y | Y | N | insp | no: reading zeros (FILE-READ-03) is enough |

### 1.5 META: metadata

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| META-STAT-01 | stat: type, size, times, link count, and permissions and owner where the format stores them. Directory size follows one documented rule for every driver (0 for directories on every driver); a symlink's size is the length of its target | core | Y | Y | Y | Y | Y | all | 3.0: directory size is 0 on every driver, documented (decided 2026-09-24) |
| META-TIME-01 | Read every timestamp the format stores, at full precision and with its UTC offset. FAT: create (10 ms), modify (2 s), access (day). exFAT: 10 ms plus a UTC offset. ISO: record time with offset; RR TF create, modify, access, attributes, backup, expiration, effective, short and long form. UDF: access, modify, attribute, create (EFE). cpio: mtime in seconds | core | Y | Y | Y | Y | Y | all | 3.0 for create, modify, access, change; RR backup/expiration/effective 3.x |
| META-TIME-02 | Set each timestamp separately (utimensat). A field the format cannot store fails with Unsupported unless the value equals what the format would report after storing it (a time rounded to the field's resolution, a mode derived from attributes counts as stored); VOL-CAPS-01 says which fields are stored | core | Y | Y | B | Y (rw) B | B | all | 3.0: one rule for every unstorable field (decided 2026-09-24) |
| META-TIME-03 | Treat FAT times as local time with a configurable UTC offset, as Windows, mtools and Linux vfat do; exFAT reads and writes its offset fields | core | Y | Y | - | - | - | all | 3.0: decision D10 |
| META-PERM-01 | Read POSIX mode bits including setuid, setgid and sticky (RR PX, UDF permissions, cpio mode); FAT and exFAT derive mode from the read-only attribute | core | Y (derived) | Y (derived) | Y (RR) | Y | Y | all | 3.0 |
| META-PERM-02 | chmod: on FAT and exFAT, clearing every write bit sets READ_ONLY and adding one clears it; any other bit change fails with Unsupported (rule of META-TIME-02) | core | Y (partial) | Y (partial) | B | Y (rw) B | B | ker, host | 3.0: integrate-20 (today a silent no-op) |
| META-OWNER-01 | Read uid and gid (RR PX, UDF, cpio); "no owner" is distinct from 0 | core | N | N | Y | Y | Y | all | 3.0 |
| META-OWNER-02 | chown on a format without owners fails with Unsupported unless the value equals the reported owner (rule of META-TIME-02); a format that reports no owner refuses every owner; host copy helpers strip unstorable fields using VOL-CAPS-01 | core | N | N | B | Y (rw) B | B | ker | 3.0: policy only |
| META-ATTR-01 | Read DOS attributes (read-only, hidden, system, archive; volume and directory are implied); ISO hidden flag; UDF hidden and system flags | core | Y | Y | Y (hidden) | Y | - | all | 3.0 |
| META-ATTR-02 | Set DOS attributes; the archive bit is set on write and rename, as Windows does | extra | Y | Y | B | Y (rw) | - | host, emb | 3.0 |
| META-NLINK-01 | Report the link count: RR PX nlink, UDF link count, cpio nlink; 1 on FAT and exFAT; a directory count of 1 means "not counted" | core | Y (1) | Y (1) | Y | Y | Y | ker, insp | 3.0 |
| META-INO-01 | A stable node identity: NodeId unchanged by rename while the node is pinned; every name of a hard link has the same id (RR via PX serial or data extent, UDF via the ICB, cpio via dev and ino) | core | Y | Y | Y | Y | Y | ker, insp | 3.0: A7 |
| META-INO-02 | A generation number, so a reused NodeId is not mistaken for the old node (FUSE) | core | Y | Y | - | Y | - | ker | 3.0: decided 2026-09-24; FUSE needs it |
| META-DEV-01 | Report the device number of a char or block device node (RR PN, UDF device specification EA, cpio rdev) through the common metadata | core | N | N | Y | Y | Y | ker, insp, bld | 3.0: decided 2026-09-24; cpio and RR extraction and FUSE stat need it |
| META-ALLOC-01 | Report allocated size (st_blocks), which differs from the length for sparse, preallocated and embedded files | core | Y | Y | Y | Y | - | ker | 3.0: decided 2026-09-24; st_blocks for FUSE |
| META-NAME-01 | Names are bytes: a name that is not valid UTF-8 (RR, cpio, UDF CS0 with odd code points) can be listed, looked up and opened; decoding to text is a separate fallible step | core | N (UTF-16) | N | Y | Y | Y | insp, host | 3.0: inspect-5 (path helpers are `&str` only) |
| META-NAME-02 | FAT 8.3 names: decode through an OEM code page (CP437 by default on the host; others possible), keep names with high bytes distinct, set NT lowercase flags when the long name is only a case change | core | Y | - | - | - | - | all | 3.0: ASCII and CP437 built in, plus a public code page trait so users add their own (decided 2026-09-24) |
| META-NAME-03 | Read the 8.3 alias of a long-named file | extra | Y | N | Y (primary name vs RR/Joliet name) | - | - | insp, host | 3.x: mdir-style listings, and seeing the ISO primary name under an RR view |
| META-NAME-04 | Set a chosen 8.3 alias for a new file | extra | Y | - | - | - | - | bld | no: Windows cannot either; generated aliases are enough |
| META-RAW-01 | Locate the on-disk record of a node (block, byte offset, length) for the FAT entry and LFN slots, exFAT entry set, ISO directory record, UDF FE/EFE, cpio header; decoding is done with the raw crates | extra | Y | Y | Y | Y | Y | insp | 3.0: location only; decoding through the raw crates (decided 2026-09-24) |

### 1.6 LINK: symlinks, hard links, special files

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| LINK-SYMLINK-01 | Read a symlink target: RR SL with several components, root, `.` and `..` flags, split across SL entries and CE areas; UDF path components; cpio target as data | core | N | N | Y | Y | Y | all | 3.0 |
| LINK-SYMLINK-02 | Create a symlink on a mounted volume | core | N | N | - | Y (rw) | - | ker | 3.x with UDF write; FAT and exFAT must refuse with Unsupported |
| LINK-SYMLINK-03 | Resolve a path following symlinks (POSIX), with a loop limit giving an ELOOP-like error; the default resolver is lexical | core | - | - | Y | Y | Y | ker, host | 3.0 |
| LINK-SYMLINK-04 | Resolve a path without following the last component (lstat, O_NOFOLLOW) | core | - | - | Y | Y | Y | ker, host | 3.0: std::fs `symlink_metadata` parity and extraction need it (decided 2026-09-24) |
| LINK-HARD-01 | Read hard links: names that share one node, with nlink and the data readable through every name (cpio newc stores data only on the last name) | core | N | N | Y (RR) | Y | Y | insp, bld | 3.0 |
| LINK-HARD-02 | Create a hard link on a mounted volume | core | N | N | - | Y (rw) | - | ker | 3.x: design 4.14 `link` |
| LINK-SPECIAL-01 | Read the type of char and block devices, FIFOs and sockets (RR PX and PN, UDF file type, cpio mode) | core | N | N | Y | Y | Y | ker, insp | 3.0 |
| LINK-SPECIAL-02 | Create a device node, FIFO or socket on a mounted volume (mknod) | core | N | N | - | Y (rw) | - | ker | 3.x: design 4.14 `NewNode::Fifo`, `Socket` |

### 1.7 XATTR: extended attributes, named streams, ACLs

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| XATTR-LIST-01 | List and read extended attributes: RR AAIP `AL` entries (xorriso --xattr), UDF extended attributes (implementation-use and application-use EAs) | core | N | N | Y | Y | N | insp, host | 3.x: design 4.14 lists it as additive; few users |
| XATTR-SET-01 | Set and remove extended attributes on a mounted volume | core | N | N | - | Y (rw) | N | host | no: no writable Hadris format stores them except UDF, and UDF write is 3.x |
| XATTR-STREAM-01 | List and read named streams (UDF 2.00+ stream directory, macOS resource forks) | extra | N | N | N | Y | N | insp | 3.x: forensics on Mac-authored UDF; reads only |
| XATTR-ACL-01 | Read POSIX ACLs (AAIP) and UDF permission EAs beyond mode bits | extra | N | N | Y | Y | N | insp | no: niche; the mode bits are enough for every listed user |
| XATTR-BUILD-01 | Write xattrs or ACLs into a built image (ISO AAIP, UDF EA) | builder | - | - | Y | Y | N | bld | no: no finding asks for it; images for booting do not use it |

### 1.8 BOOT: boot structures

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| BOOT-ET-READ-01 | Read the El Torito catalog: validation entry and checksum, default entry, section headers and entries (platform x86, EFI, PPC, Mac), emulation type, load segment, sector count, load RBA; extension entries skipped | extra | - | - | Y | - | - | insp, bld | 3.0 |
| BOOT-ET-READ-02 | Extract each boot image's bytes, as geteltorito and xorriso -report_el_torito do. The size is ambiguous for no-emulation entries (sector count × 512 vs the file behind the RBA) and the rule used must be documented | extra | - | - | Y | - | - | insp, bld | 3.0: remastering needs the images out |
| BOOT-ET-READ-03 | Read and check boot info tables in boot images (isolinux, GRUB 2 variant) | extra | - | - | Y | - | - | insp | 3.x: verification only |
| BOOT-ET-WRITE-01 | Write a BIOS no-emulation entry with a chosen load size and optional boot info table patching (isolinux, GRUB 2) | builder | - | - | Y | - | - | bld | 3.0 |
| BOOT-ET-WRITE-02 | Write several entries in sections (BIOS plus UEFI 0xEF), with the catalog hidden or at a chosen path | builder | - | - | Y | - | - | bld | 3.0 |
| BOOT-ET-WRITE-03 | Write floppy (1.2, 1.44, 2.88 MB) and hard-disk emulation entries, refusing a size that does not match the emulation | builder | - | - | Y | - | - | bld | 3.0 for validation (osdev-2); emulation writing already exists |
| BOOT-ET-WRITE-04 | Warn or fail on inconsistent boot setups: two EFI entries with GPT but no ESP, bootstrap over 446 bytes, load size past the image | builder | - | - | Y | - | - | bld | 3.0: osdev-1, -2, -3 (A11) |
| BOOT-HYB-01 | Write an isohybrid MBR: bootstrap code up to 446 bytes and a partition entry covering the image, so the ISO boots from USB | builder | - | - | Y | - | - | bld | 3.0 |
| BOOT-HYB-02 | Write a GPT in the system area (protective or hybrid MBR, backup GPT at the end, ESP pointing at the El Torito EFI image) | builder | - | - | Y | - | - | bld | 3.0 |
| BOOT-HYB-03 | Append partitions after the ISO data (xorriso -append_partition, typically a FAT ESP image) and list them in the MBR or GPT | builder | - | - | Y | - | - | bld | 3.0: common Linux distro layout; osdev-6 shows remastering such images fails today |
| BOOT-HYB-04 | Write an Apple Partition Map in the system area (Mac booting of hybrid images) | builder | - | - | Y | - | - | bld | 3.x: old Macs only; additive if the hybrid layout options stay non_exhaustive |
| BOOT-HYB-05 | Read the system area of an ISO: MBR, GPT, APM and the bootstrap bytes | extra | - | - | Y | - | - | insp, bld | 3.0: inspection and remastering (xorriso -report_system_area) |
| BOOT-HYB-06 | Write MIPS, SPARC, HPPA or Alpha boot blocks | builder | - | - | Y | - | - | bld | no: dead platforms |
| BOOT-VBR-01 | Install boot code into a FAT or exFAT volume boot record while keeping the BPB (syslinux-style), with exFAT boot checksum recomputed | extra | Y | Y | - | - | - | bld | 3.x: osdev feature idea (S); additive option |
| BOOT-VBR-02 | Write a payload into the FAT reserved sectors or exFAT extended boot sectors (stage 2 loader) | extra | Y | Y | - | - | - | bld | 3.x: osdev feature idea; additive |
| BOOT-VBR-03 | Read the raw boot sector and boot code of a volume | extra | Y | Y | Y (system area) | - | - | insp | 3.0 through the raw crates |
| BOOT-UDF-01 | Write or read the UDF boot descriptor | extra | - | - | - | Y | - | bld | no: no known firmware uses it |

### 1.9 BUILD: image construction

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| BUILD-TREE-01 | Build an image from one in-memory tree of files, directories, symlinks, hard links and device nodes with metadata; the same tree feeds every writer | builder | Y (via mount) | Y (via mount) | Y | Y | Y | bld, host | 3.0 |
| BUILD-TREE-02 | Build a tree from a host directory with a symlink policy (keep, follow, skip), excludes, per-entry error handling, content read lazily, and names that are not UTF-8 | builder | - | - | - | - | - | bld, host | 3.0 for lazy content, error policy and an exclude filter (decided 2026-09-24) |
| BUILD-TREE-03 | Build a tree from an existing image of any format (ISO, UDF, cpio, FAT) for remastering or conversion, keeping hard links and devices | builder | Y | Y | Y | Y | Y | bld, host | 3.0 for ISO (Session), generic drivers and cpio (`Tree::from_cpio`, decision 4) (decided 2026-09-24) |
| BUILD-TREE-04 | Edit a tree: insert, remove and replace nodes (needed by SESSION-APPEND-01); bulk helpers such as walk, clamp mtimes, set owners to 0, clone | builder | - | - | - | - | - | bld | 3.0 for insert, remove, replace; bulk helpers 3.x (integrate-26) (decided 2026-09-24) |
| BUILD-REPRO-01 | Reproducible output: fixed clock or SOURCE_DATE_EPOCH, mtime clamping, deterministic order, serials, UUIDs, GUIDs and inode numbers. Two runs give identical bytes | builder | Y | Y | Y | Y | Y | bld | 3.0: osdev UC4, distro builds |
| BUILD-PLAN-01 | Plan without writing: total size and layout, and every option error found before the output is touched | builder | Y (format) | Y (format) | Y | Y | Y | bld | 3.0: dx-101 (a failed create destroyed the existing file) |
| BUILD-REPORT-01 | A report of every lossy conversion with its tree path: renamed, truncated, dropped metadata, skipped nodes; inherent format limits reported once per kind, not per file | builder | Y | Y | Y | Y | Y | bld, host | 3.0: dx-8, integrate-11 (warning floods) |
| BUILD-REPORT-02 | The report gives each file's final location (LBA and length) for boot tools and bridge writers | builder | Y | Y | Y | Y | Y (offset) | bld | 3.0 |
| BUILD-ISO-NAME-01 | ISO levels 1, 2 and 3 with name mapping: d-characters, `.` always present and `;1` added, extension kept on truncation, the last dot starts the extension, mapping per character not per byte, collisions deduplicated with a warning | builder | - | - | Y | - | - | bld | 3.0: A8, A12, dx-122 |
| BUILD-ISO-NAME-02 | Relaxed primary naming (lowercase kept, version omitted, deeper than 8 levels, long names) as in mkisofs -relaxed options | builder | - | - | Y | - | - | bld | 3.x: legacy readers only; RR or Joliet is the real answer |
| BUILD-ISO-1999-01 | ISO 9660:1999 enhanced volume descriptor tree (207-byte names) | builder | - | - | Y | - | - | bld | 3.0 (already written) |
| BUILD-ISO-JOLIET-01 | Joliet with UCS-2 names up to 64 characters, forbidden characters (`* / : ; ? \`) mapped, a warning for every truncation and case-only collision | builder | - | - | Y | - | - | bld | 3.0: integrate-14 |
| BUILD-ISO-JOLIET-02 | Joliet names beyond the BMP (UTF-16 surrogates) and names up to 103 characters (joliet-long) | builder | - | - | Y | - | - | bld | 3.x: design 5.2 feature work |
| BUILD-ISO-RR-01 | Rock Ridge with SP, ER, PX (with serial), TF, NM, SL, PN, CE, on every record of a multi-extent file | builder | - | - | Y | - | - | bld | 3.0: A6 (bsdtar fails on >4 GiB files today) |
| BUILD-ISO-RR-02 | Relocate directories deeper than 8 levels with CL, PL, RE (or refuse), and hide the relocation directory from the RR view and from extraction | builder | - | - | Y | - | - | bld | 3.0: dx-102, dx-103 |
| BUILD-ISO-ORDER-01 | Control file placement: sort weights (mkisofs -sort) and per-file data alignment | builder | - | - | Y | - | - | bld | 3.x: optical seek time and boot images that need alignment; additive |
| BUILD-ISO-BIG-01 | Write a file over 4 GiB as a multi-extent file at level 3; at levels 1 and 2 fail with an error naming the file and level (not ImageTooLarge) | builder | - | - | Y | - | - | bld | 3.0: integrate-7 |
| BUILD-ISO-ZISOFS-01 | Write zisofs-compressed files (RR ZF) | builder | - | - | Y | - | - | bld | 3.x: design 5.2; only Linux reads it |
| BUILD-ISO-DEDUP-01 | Store identical file contents once (shared extents without being hard links) | builder | - | - | Y | Y | N | bld | no: small savings; hard links already share |
| BUILD-ISO-PAD-01 | Padding and volume size: tail padding so small images read in every tool, and a PVD volume space size that covers the whole image including a UDF bridge tail | builder | - | - | Y | Y | - | bld | 3.0: A9, dx-127 |
| BUILD-ISO-IDS-01 | Set every descriptor identifier and date (system, volume, volume set, publisher, preparer, application, copyright, abstract, bibliographic, expiration, effective) with length and charset checks | builder | - | - | Y | Y | - | bld | 3.0 |
| BUILD-BRIDGE-01 | Write an ISO and UDF bridge image in which both trees share the file data | builder | - | - | Y | Y | - | bld | 3.0: decision S3 (`hadris_udf::write_bridge`) |
| BUILD-UDF-REV-01 | Choose the UDF revision: 1.02, 1.50, 2.00, 2.01 written as specified; 2.50 and 2.60 (metadata partition) refused until supported | builder | - | - | - | Y | - | bld | 3.0 for up to 2.01; 2.50+ 3.x |
| BUILD-UDF-BS-01 | Write UDF with 512 or 4096-byte blocks for hard disks and USB media | builder | - | - | - | Y | - | bld, host | 3.x: optical images use 2048; disk UDF is rare |
| BUILD-UDF-EFE-01 | Write Extended File Entries at 2.00 and later, storing creation times; embed small files in the ICB | builder | - | - | - | Y | - | bld | 3.0 for EFE creation times (integrate-11); embedding 3.x |
| BUILD-UDF-DEV-01 | Write device nodes into UDF (device specification EA) | builder | - | - | - | Y | - | bld | 3.x: needs EA writing; rare on UDF |
| BUILD-UDF-META-01 | Write a UDF 2.50+ metadata partition with its mirror (Blu-ray) | builder | - | - | - | Y | - | bld | 3.x: needed for 2.50/2.60 and BD-ROM |
| BUILD-UDF-VAT-01 | Write VAT (CD-R incremental) or sparable (CD-RW) partitions | builder | - | - | - | Y | - | bld | no: packet-writing media are obsolete |
| BUILD-CPIO-FMT-01 | Write newc, crc (checksum computed) and odc; refuse fields that do not fit (odc 18-bit ino, newc 32-bit size) with an error naming the path | builder | - | - | - | - | Y | bld | 3.0 |
| BUILD-CPIO-FMT-02 | Write old binary cpio | builder | - | - | - | - | Y | bld | no: obsolete; reading it is enough |
| BUILD-CPIO-STREAM-01 | Stream a cpio archive entry by entry with streamed content, without building a whole tree, then finish with the trailer | builder | - | - | - | - | Y | bld, host | 3.0 |
| BUILD-CPIO-INITRAMFS-01 | Initramfs conventions: parents before children, `/dev/console` char device, hard links with data on the last name, 4-byte alignment, trailer; the kernel boots it | builder | - | - | - | - | Y | bld | 3.0 |
| BUILD-CPIO-CAT-01 | Concatenate archives: early microcode segment then main archive, each with its own trailer | builder | - | - | - | - | Y | bld | 3.0: standard x86 initrd layout |
| BUILD-CPIO-ZIP-01 | Compress the archive (gzip, zstd, xz) | builder | - | - | - | - | Y | bld | no: compose with a compressor stream |
| BUILD-FAT-01 | Build a FAT or exFAT image from a tree in one call, optionally sized to fit ("the smallest FAT16 ESP holding this tree") | builder | Y | Y | - | - | - | bld | 3.0: same `write(dev, &tree, &opts) -> Report` shape as the other writers (decided 2026-09-24); fit-to-size planner 3.x |
| BUILD-ESP-01 | Scenario: build a FAT ESP image from a tree, use it as the El Torito EFI image and as an appended GPT partition of an ISO in one run; the image boots in OVMF and from USB | builder | Y | - | Y | - | - | bld | 3.0: every distro-style build; checks that the FAT and ISO builders compose, adds no API (decided 2026-09-24) |
| BUILD-HOST-01 | Import a host directory into a mounted volume, with a symlink policy (skip, follow, error), excluding the output file itself, keeping directory times | builder | Y | Y | - | Y (rw) | - | bld, host | 3.0 as a composition test only: host tree (BUILD-TREE-02, excluding the output) plus copy_tree (BUILD-COPY-01); no API of its own (decided 2026-09-24) |
| BUILD-COPY-01 | Copy a tree between any two filesystems (ISO to exFAT, FAT to cpio). Fields the target cannot store (VOL-CAPS-01) are stripped and nodes it cannot hold are skipped, all listed in the same Report the writers return (BUILD-REPORT-01) | builder | Y | Y | B | Y | B | host, bld | 3.0 with strip-and-report; other policies 3.x (decided 2026-09-24) |

### 1.10 SESSION: multi-session, modification, append

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| SESSION-READ-01 | Read the newest session of a multi-session image, including files whose data sits in earlier sessions | builder | - | - | Y | Y (VAT) | Y (segments) | insp, bld | 3.0 for ISO |
| SESSION-READ-02 | Open a chosen earlier session by its start block (mount -o session=, xorriso -load sbsector). Listing sessions by scanning is not offered | builder | - | - | Y | - | - | insp | 3.x; scanning `no` (decided 2026-09-24) |
| SESSION-APPEND-01 | Append a new ISO session that adds, removes and replaces files, reuses the previous extents, and writes new descriptors after the old volume (growisofs -M) | builder | - | - | Y | - | - | bld | 3.0: design 5.2 |
| SESSION-REWRITE-01 | Rewrite an image in place with a changed tree, keeping the hybrid boot data and appended partitions and updating the backup GPT | builder | - | - | Y | - | - | bld | 3.0 |
| SESSION-BOOT-01 | Remaster boot setup: replace a boot image or change entries, with the catalog, load sizes and boot info tables rebuilt; the existing El Torito setup can be read back as options | builder | - | - | Y | - | - | bld | 3.0: A10, osdev-4, osdev-5 (the old loader still boots today) |
| SESSION-UDF-01 | Append a UDF session on write-once media (VAT) | builder | - | - | - | Y | - | bld | no: packet-writing media are obsolete |
| SESSION-CPIO-01 | Append entries to an existing cpio archive (cpio -A: drop the trailer, append, write a new trailer) | builder | - | - | - | - | Y | bld | 3.x: cheap, rare |
| SESSION-CPIO-02 | Read concatenated archives across trailers, with zero padding between segments and a different format per segment | builder | - | - | - | - | Y | bld, insp | 3.0: every x86 initrd with microcode |

### 1.11 CHECK and REPAIR

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| CHECK-FSCK-01 | Read-only check with findings. FAT: boot sector, backup, reserved entries, FSInfo, free count, FAT copies, invalid/broken/cyclic chains, size mismatches, cross-links, lost clusters, dot entries, names, LFN checksum, orphan LFN, dirty flag. exFAT adds boot checksum, up-case table, set checksums, name hash, VDL, bitmap | extra | Y | Y | - | - | - | host, insp, emb | 3.0 |
| CHECK-FSCK-02 | Findings name their location as a path, print with Display, and cross-links come as runs with the other owner named | extra | Y | Y | Y | Y | Y | insp, host | 3.0 for Display and path (inspect-13, dx-117); owner coalescing 3.x (inspect-14) |
| CHECK-FSCK-03 | Check with bounded memory and no allocator (caller bitmap window, several passes) | extra | Y | Y | - | - | - | emb | 3.0 |
| CHECK-FSCK-04 | Check a volume that will not mount (damaged boot sector, bad root) and explain why | extra | Y | Y | Y | Y | - | insp | 3.0: check runs on the raw I/O layer without mounting, and the error detail explains why mount fails (decided 2026-09-24) |
| CHECK-REPAIR-01 | Repair: free or recover lost chains (FSCK0000.REC), cut cross-links, fix sizes, copy the good FAT over the others, fix FSInfo and PercentInUse, fix set checksums, clear dirty; fsck.fat -a and fsck.exfat -y parity | extra | Y | Y | - | - | - | host, emb | 3.x: design 5.1; large and risky |
| CHECK-REPAIR-02 | Restore the boot sector from the backup (FAT32 sector 6, exFAT backup region) | extra | Y | Y | - | Y (reserve VDS) | - | host | 3.x: goes with repair |
| CHECK-ISO-01 | Verify an ISO: descriptors, both-endian fields, path tables against directories, extents inside the volume, SUSP and RR structure, catalog checksum and entries, load sizes, truncation | extra | - | - | Y | - | - | bld, insp | 3.0 as a library call (CI of image builders) |
| CHECK-UDF-01 | Verify a UDF volume: descriptor tag checksums and CRCs, both anchors, main and reserve VDS agree, LVID counts, space bitmap against allocated extents, unique ids | extra | - | - | - | Y | - | bld, insp | 3.x: udfinfo covers most CI needs |
| CHECK-CPIO-01 | Verify a cpio archive: crc checksums, header fields, padding, trailer, names (no absolute or `..` paths) | extra | - | - | - | - | Y | bld, insp | 3.0: cheap, mostly exists in the reader (dx-130) |
| CHECK-COMPARE-01 | Compare an image's contents with its source tree or host directory, and ISO against UDF in a bridge image | extra | Y | Y | Y | Y | Y | bld | 3.x: CLI level |
| CHECK-BAD-01 | Never allocate clusters marked bad (FAT 0xFFF7 family, exFAT bad cluster) | core | Y | Y | - | - | - | all | 3.0 |
| CHECK-BAD-02 | Mark clusters bad from a surface scan | extra | Y | Y | - | - | - | host | no: device-level job |

### 1.12 HOST: host integration and extraction

| ID | Action (edge case that matters) | Class | F | X | I | U | C | Users | Want |
|---|---|---|---|---|---|---|---|---|---|
| HOST-EXTRACT-01 | Extract a whole image to a host directory, safe against absolute names, `..`, drive prefixes and writing through existing symlinks; keep modes and times; errors name the path | extra | Y | Y | Y | Y | Y | host, insp, bld | 3.0 |
| HOST-EXTRACT-02 | Extraction policy: skip or report per-entry failures, overwrite policy, restore owners when privileged, create device nodes when privileged | extra | Y | Y | Y | Y | Y | host, insp | 3.x: defaults decided in 3.0 (skip devices with a warning) |
| HOST-EXTRACT-03 | Extract one path (file or subtree) | extra | Y | Y | Y | Y | Y | host | 3.0 |
| HOST-ERRNO-01 | Map every error kind to an errno (Unsupported maps to EOPNOTSUPP, never ENOSYS, which FUSE reads as "never call again"; EROFS, ENOTEMPTY, ELOOP, ENAMETOOLONG) | core | Y | Y | Y | Y | Y | ker | 3.0: integrate-17, promised in docs |
| HOST-FUSE-01 | A FUSE or VFS adapter over any driver: inode mapping with root = 1, lookup and forget counts, readdir offsets, open handles | extra | Y | Y | Y | Y | Y | ker, host | 3.x: integrate UC1 (every user writes about 450 lines) |
| HOST-CONC-01 | Share one mounted volume between threads or tasks with any number of open handles; dropping handles while holding the volume lock does not deadlock | core | Y | Y | Y | Y | - | host, ker | 3.0 |
| HOST-CONC-02 | Serve concurrent reads on a read-only volume (ISO, UDF) without serializing every call | extra | - | - | Y | Y | - | host | 3.x: server E16; `FileSystem` allows it later |

---

## 2. Mounted operation set vs build-then-read

A mounted filesystem, whether in a kernel VFS, FUSE or 9P, uses a small, closed set of operations:

`lookup, stat, setattr, readdir, create, mkdir, unlink, rmdir, rename, link, symlink, readlink, open, read, write, truncate, sync, statfs, xattr`

(plus mknod and the forget bookkeeping). Every `core` row above is one of these, or a precondition of one: capabilities, parent, device recovery. The `FsDriver` node API in design 4.3 already covers the set, and 4.14 lists the missing pieces (link, mknod kinds, exchange, xattr, forget_n, generation, allocated, device) as additive.

The formats split in two:

- **FAT and exFAT** are true mounted filesystems. Nearly every core row applies in both directions. Their extras are volume maintenance: format, label, serial, check, repair, resize, boot code, geometry.
- **ISO 9660, UDF (as Hadris ships it) and cpio** are build-then-read formats. The read half of the core set applies (lookup, stat, readdir, readlink, open, read, statfs). The write half is replaced by the builder class: build a tree, plan, write, report, and for ISO a session that edits a tree and writes it again. For UDF the write half becomes core once read-write mount exists (VOL-MOUNT-05). cpio is a stream: its "read" side is a sequential entry iterator, and mounted-style random access needs an index.

Counts by class (section 1): 215 actions, of which 85 are core, 61 builder and 69 extra; 151 are wanted in 3.0, 42 in 3.x, and 22 are `no`. Many builder and extra rows exist for one format only, which is why the design keeps them native (inherent methods and options) and keeps the shared trait closed.

Implication for the prototype API: the shared trait needs only the core rows. Builder rows need one tree type, one options-plus-report shape per writer, and a session type. Extra rows are per-format inherent methods or raw-crate functions.

---

## 3. Non-functional constraints

Each constraint gets its own conformance or CI check. Numbers come from the embedded, server and inspect findings.

| ID | Constraint | Want |
|---|---|---|
| NF-NOALLOC-01 | FAT12/16/32 read and write on a device with no global allocator (embedded API) | 3.0 |
| NF-NOALLOC-02 | exFAT read with no allocator; exFAT write with no allocator | read 3.0, write 3.x (design 4.15) |
| NF-NOALLOC-03 | ISO and UDF read with no allocator (lookup, readdir, read, readlink, El Torito catalog read) | 3.0 |
| NF-NOALLOC-04 | cpio streaming read with no allocator | 3.0 |
| NF-NOALLOC-05 | cpio newc write with no allocator (fixed-size headers, caller content) | 3.x |
| NF-NOSTD-01 | Every format crate builds `no_std`; std only adds host adapters, host paths and SystemClock | 3.0 |
| NF-TARGET-01 | Builds for thumbv6m-none-eabi and riscv32imc-unknown-none-elf (no compare-and-swap) and thumbv7em-none-eabihf, checked in CI | 3.0 |
| NF-STACK-01 | Embedded API: mount under 2 KB of stack and under 2 KB of RAM on thumbv7em (design 4.15 target). Reference: embedded-sdmmc main frame 1208 B | 3.0 |
| NF-STACK-02 | No single frame over 1 KB in the embedded API's name, rename and create paths | 3.0 |
| NF-STACK-03 | Shared-tier sync stack on thumbv7em does not grow; async stays within about 1.7× of sync (measured: 26 KB sync vs 45 KB block_on async) | 3.0: keep sync macro-generated |
| NF-FLASH-01 | Embedded FAT read/write binary within about 1.5× of embedded-sdmmc (13852 B text): target under 20 KB text on thumbv7em, opt-level s, LTO | 3.0 target |
| NF-FLASH-02 | No Unicode case tables linked unless the caller asks for Unicode folding (the 12.2 KB `to_uppercase` rodata) | 3.0 |
| NF-RAM-01 | Driver state size documented and bounded: FAT shared driver (4496 B with 4 slots, 7376 B with 64 today), exFAT 12584 B | 3.0 (document), 3.x (shrink) |
| NF-BUF-01 | Embedded API works with 512-byte buffers; no fixed 4 KiB buffer when the device block is 512 bytes | 3.0 |
| NF-MODE-01 | Sync and Send-async parity: every action above exists in both modes, except host helpers (sync only) and documented mode-specific locks; checked in CI | 3.0 |
| NF-MODE-02 | Async futures are `Send` whenever the device is `Send` | 3.0 |
| NF-MODE-03 | The embedded API has sync and non-Send async variants | 3.0 |
| NF-CANCEL-01 | Dropping any future at any await point leaves a FAT or exFAT volume that is consistent after the next sync: fsck.fat and fsck.exfat clean, no leaked clusters, FAT copies equal | 3.0 |
| NF-CRASH-01 | Power loss after any device write leaves at worst lost clusters, never cross-links or corrupt entry sets; an interrupted operation is finished by the next write or sync | 3.0 |
| NF-ATOMIC-01 | A failed operation leaves the volume unchanged in memory and on disk (the contract's rejection scenarios) | 3.0 |
| NF-ERR-01 | Errors keep kind, a static context, a location (byte, block, cluster) and a format detail code, with no allocation, across every crate boundary | 3.0 |
| NF-ERR-02 | "Not this format" (NotRecognized) is a different kind from "damaged" (Corrupt) | 3.0 |
| NF-ERR-03 | Display never repeats the source's text | 3.0 |
| NF-ERR-04 | Host, writer and extraction errors carry the tree path and host path | 3.0 |
| NF-ERR-05 | Errors format with defmt on embedded | 3.x |
| NF-PANIC-01 | No panic, hang or unbounded memory on any input bytes: fuzzed, cycles detected, bounded loops (reference: 31240 corrupted runs, 0 panics, max RSS 16 MB) | 3.0 |
| NF-PERF-01 | Directory create and lookup take time linear in directory size (reference: 10k creates took 18.8 s on FAT and 24.4 s on exFAT) | 3.0 |
| NF-PERF-02 | Contiguous clusters go to the device in one multi-block call; FAT sector updates are batched | 3.0 |
| NF-PERF-03 | Block cache costs O(1) per hit or miss (Cache(65536) is 230 times slower today) | 3.0 |
| NF-PERF-04 | Path walks cost per component, not per directory size (inspect-4: 5.96 s vs 0.9 ms) | 3.0 |
| NF-SCALE-01 | Open-node limit is unbounded on the host tier (default 64 slots broke servers at 63 open files) | 3.0 |
| NF-INTEROP-01 | Every image Hadris writes passes the native tools: fsck.fat, fsck.exfat (both exfatprogs and macOS fsck_exfat), xorriso, isoinfo, bsdtar, udfinfo, GNU cpio, Linux mount, and booting in SeaBIOS and OVMF for boot images | 3.0 |
| NF-INTEROP-02 | Images written by the native tools read correctly (mkfs.fat and mtools, mkfs.exfat, Windows, xorriso and genisoimage, mkudffs, GNU cpio and the kernel's gen_init_cpio) | 3.0 |
| NF-DET-01 | With a fixed clock and the same input, output is byte-identical across runs, hosts and modes | 3.0 |
| NF-STABLE-02 | UDF read-write mount (VOL-MOUNT-05, VOL-FORMAT-06, LINK-HARD-02, FILE-UNLINK-02 on UDF) can be added in 3.x purely additively: the 3.0 UDF type and the mounted-filesystem API need no signature change. UDF writes come through a new entry point (for example `mount_writable`) and `mount` stays read-only in every version. The prototype must show it | 3.0 |
| NF-STABLE-01 | Public API follows design section 2 (R1 to R11): non_exhaustive enums, private fields, no cfg on public shapes, features strictly additive and never switching behaviour | 3.0 |

---

## 4. Decisions (2026-09-24)

1. UDF read-write mount stays 3.x, but only if it can land additively (NF-STABLE-02).
2. Deleting an open file returns Busy in 3.0; POSIX orphans (FILE-UNLINK-02) are 3.x.
3. Device number, generation and allocated size (META-DEV-01, META-INO-02, META-ALLOC-01) are 3.0.
4. cpio gets the stream reader and `Tree::from_cpio` in 3.0; a read-only cpio driver is 3.x.
5. FAT and exFAT get a tree writer with the shared writer shape in 3.0 (BUILD-FAT-01); fit-to-size is 3.x.
6. Changing the FAT and exFAT serial is 3.0 (VOL-SERIAL-02).
7. The FAT dirty flag is set on the first write, not on mount (VOL-MOUNT-04).
8. Legacy El Torito emulation stays (BOOT-ET-WRITE-03). APM is 3.x, provided the hybrid options stay non_exhaustive (BOOT-HYB-04).
9. Streaming ISO and UDF output is 3.x (IO-STREAM-02).
10. Opening an earlier session at a given block is 3.x; listing sessions by scanning is `no` (SESSION-READ-02).
11. Resize is `no` (VOL-RESIZE-01).
12. ASCII and CP437 built in, plus a public code page trait in 3.0 (META-NAME-02).
13. No extended attributes in 3.0 (XATTR-*).
14. Repair is 3.x (CHECK-REPAIR-01); check-only in 3.0.
15. TRIM and preallocation are 3.x (IO-TRIM-01, FILE-ALLOC-01, FILE-ALLOC-02).
16. Directory size is 0 on every driver (META-STAT-01).

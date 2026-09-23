# V3 trait review (migration step 6)

Status: findings for discussion. Nothing in `hadris-fs` or `hadris-fat` was
changed; every proposal below needs a decision first.

Step 6 of [the V3 design](v3-api-design.md#6-migration-plan) freezes the
`hadris-fs` traits before ISO, UDF, NTFS and exFAT implement them. This review
tested them against four users:

- `FatFs` (`crates/block/hadris-fat/src/fatfs.rs`), the reference driver.
- The conformance adapter (`tests/src/fat/generic.rs`), the in-memory test
  driver (`crates/core/hadris-fs/tests/common/mem_fs.rs`) and the FAT CLI
  (`crates/tools/hadris-fat-cli/src/app.rs`).
- Experiment E3's `DynFileSystem` sketch (branch `exp/v3-second-format`,
  `experiments/lock-placement/src/dyn_fs.rs`).
- A new prototype FUSE adapter, `experiments/fuse-prototype`, mounted for
  real and driven with shell tools.

File and line references are to `next` at `af888a4`.

## 1. The FUSE prototype

`experiments/fuse-prototype` is a detached package on `fuser` 0.18.0. Its
`Adapter` implements each FUSE request as a method over any sync
`hadris_fs::sync::FileSystem` and returns `Result<_, Errno>`; `Fuse` is the
`fuser::Filesystem` glue. It sits on `FileSystem`, not `FsDriver`, because
`fuser` 0.18 takes `&self` in every request and requires `Send + Sync`, which
is exactly what `Volume` provides. The mount uses
`Volume<FatFs<File, HeapTable, SystemClock>, StdMutex>` with four `fuser`
worker threads.

Every workaround in the adapter is marked `GAP:` in `src/lib.rs`.
`tests/ops.rs` replays the kernel's request sequences against `Adapter`
without a mount (15 tests, macOS host). `tests/dyn_check.rs` checks that the
sync `FileSystem` is usable as a trait object.

### 1.1 Mount results

The real mount worked. `scripts/docker.sh` runs a privileged
`rust:1-bookworm` container on OrbStack (Linux 7.0.14, aarch64) with
`/dev/fuse`, installs `fuse3` and `dosfstools`, builds the adapter, formats a
256 MiB FAT32 image with `hadris_fat::sync::format` and runs
`scripts/scenarios.sh`. GAP marks a difference from POSIX that follows from
the current contract or from FAT; it is recorded, not failed.

| Scenario | Result | Notes |
|---|---|---|
| `mkdir -p` trees | PASS | 7 levels |
| `cp -r` in, `cat`, `cmp` | PASS | 0 B to 3 MB files, nested directories |
| `cp -p` | PASS | chmod and chown succeed and are ignored |
| `ls -la` on 3000 entries | PASS | More than 30 readdir buffers. `ls -f` 10 ms, `ls -la` 1.8 s (see F8) |
| `mv` rename, across directories, directory move | PASS | |
| `mv` over an existing file, `mv -T` over an empty directory | PASS | Needs the pin workaround (F1) |
| `renameat2` `RENAME_NOREPLACE`, `RENAME_EXCHANGE` | PASS | `EEXIST`; `EINVAL` for exchange |
| Case-insensitive names | GAP | `cat casename.txt` finds `CaseName.TXT`. A case-only rename never reaches the driver: the kernel sees one inode under both names and returns success (Linux `vfat` behaves the same). Renaming through a temporary name works |
| `rm -r` | PASS | Needs the pin workaround (F1) |
| `truncate` shrink, grow (zero fill), to 0 | PASS | |
| `touch -m -d`, `touch -a -d` | PASS | An odd second is stored as the even second below; atime keeps only the date |
| 16 concurrent `cmp` readers | PASS | 4 FUSE threads, one `Volume` lock |
| `find` by type, name, size | PASS | |
| `df`, `stat -f` | PASS | |
| `ls -f` while another process creates 500 entries | PASS | 44 listings, no duplicates, no missing old entries, final count right |
| Inode numbers from readdir and stat | GAP | Differ after a rename frees a slot: `0x400000000000820D` vs `0x8000000000000005` (F7) |
| `rm` of an open file | GAP | `EBUSY` (F1) |
| `mv` over an open file | GAP | `EBUSY` (F1) |
| `ln -s`, `ln` | GAP | `EPERM`, FAT has neither |
| Unmount with a file open | PASS | Plain `umount` fails "target is busy"; `umount -l`, write more, close: the daemon exits, `destroy` syncs, the data is on disk |
| `fsck.fat -n` after unmount | PASS | |
| Remount, verify checksums and counts | PASS | |
| `fsck.fat -n` final | PASS | |

## 2. Friction points

Severity:

- **Must**: fix before the freeze, because changing it later breaks
  drivers, callers or the contract.
- **Should**: fix before the next format ports; cheap now, not strictly a
  break later.
- **3.x**: can be added compatibly after 3.0 under R10.

### F1. A lookup pin blocks `remove` (Must)

Evidence: `FsDriver::remove` and `FileSystem::remove` fail with `Busy` while
the node is pinned (`crates/core/hadris-fs/src/api/driver.rs:91-99`,
`:230-238`); `FatFs` checks user pins (`fatfs.rs:1046`, and `:1881` for a
replacing rename). The kernel holds a FUSE lookup on every name it has
resolved, and `unlink`, `rmdir` and `rename` are always preceded by a lookup
of the target, so without a workaround every `rm`, `rmdir` and replacing `mv`
fails with `EBUSY`. Test `driver_refuses_remove_of_a_looked_up_name` shows it.

The adapter works around it (`Adapter::unpinned`): it tracks the kernel's
lookup counts itself, drops every driver pin on the target, removes, marks
the kernel's count as dead so later `forget`s are not passed on, and on
failure looks the name up again to restore the pins. That is fragile in three
ways. It needs a lookup and a metadata call before every removal. If the
removal fails, the re-lookup can hand out a different id (FAT fallback ids),
and the kernel's inode is then stale with no way back. And it is easy to get
wrong: the first version leaked a pin on the `EBUSY` path, which only the
`unlink_of_an_open_file_is_busy` test caught.

The underlying problem is that one pin count means two things: "the caller
holds this id" (FUSE lookup, dcache) and "the node is open" (`File`,
`OpenFile`). Q3 chose `Busy` for the second; FUSE needs the first to be
cheap and non-blocking. NTFS and UDF hard links add a third case: removing
one name of a pinned node with other links should never be `Busy`.

Proposal: split the two.

- `lookup`, `create` and `parent` keep taking a pin, which never blocks
  `remove`. Removing a pinned node succeeds; the id stays valid for `forget`
  and returns `NotFound` (or `InvalidHandle`) from every other method until
  its last pin is dropped. The name is free at once.
- A new `open(node)` / `close(node)` pair (default no-op, so read-only
  drivers write nothing) marks a node as open. `remove` and a replacing
  `rename` fail with `Busy` only for an open node, and only when the name
  removed is its last link.
- `File` and `OpenFile` call `open`/`close`. The FUSE adapter calls them
  from `open`/`release`, and the `unpinned` dance disappears.

Cost: `FatFs` keeps an `opens` count and an `unlinked` flag in its node state
(`Node`, `fatfs.rs:56`); `remove` and `replace_entry` test `opens` instead of
`user_pins`; `node`/`any_node` return `NotFound` for an unlinked entry. The
`Busy` tests in `hadris-fat/tests/fatfs_write.rs` and `fatfs_async.rs` change
to hold an open instead of a pin; the conformance suite is unaffected (it
never holds a pin across a removal).
Full POSIX (reading an unlinked open file) stays out of scope, as Q3 decided.

### F2. `Metadata` is `Copy + Eq + Hash`, which rules out `extra()` (Must decide)

Evidence: `Metadata`, `SetMetadata`, `FsStats`, `Capabilities`, `DirEntry`
derive `Copy`, `PartialEq`, `Eq`, `Hash` (`meta.rs:49`, `:158`,
`caps.rs:41`, `:197`, `dir.rs:35`). Section 4.3 of the design promises an
`extra()` hook on `Metadata` for per-format types (NTFS security
descriptors, section 5.5). A hook that holds anything owned cannot be
`Copy`, and removing a derive after 3.0 is a break.

Proposal: keep the derives and drop `extra()` from the design. Per-format
metadata stays native (`FatFs::fat_attributes`, an NTFS
`security_descriptor(node)`), reached through `vol.lock()` as other
format-specific calls are. Plain fields that FUSE, `stat` and cpio need are
added as private fields with getters, which is compatible because the
fields are private and construction goes through `new` and `with_*`: see F9.
Cost: a design doc edit.

### F3. `LimitExceeded` means too many things for an errno (Must decide)

Evidence: `FatFs` returns `LimitExceeded` for a name longer than 255 UTF-16
units (`NewName::new`, `fatfs.rs:148`), a full node table (`:1013`,
`:1029`, `:2202`), and a write or `set_len` past 4 GiB - 1 (`:1160`,
`:1199`). The FUSE adapter has to pick one errno for all three
(`errno()` in `src/lib.rs`): `ENAMETOOLONG`, `ENFILE` and `EFBIG` are all
right for one cause and wrong for the others. It picks `EFBIG`, since the
kernel rejects names over 255 bytes before they arrive; a full table, which
is a real risk with `FixedTable<64>` under FUSE, then reads as "file too
large". `std::io::ErrorKind` conversion has the same problem
(`error.rs`, `LimitExceeded` to `Other`). Section 4.6 also describes a
private `context` and a `Detail` enum that `Error<E>` does not have yet
(`error.rs`, fields `kind` and `device` only), and `Error<E>` derives
`PartialEq`, so adding context later changes what `==` compares.

Proposal: add `NameTooLong` and `FileTooLarge` kinds now and keep
`LimitExceeded` for the rest (table full, path too long, a value that does
not fit a field). `ErrorKind` is non-exhaustive, so adding kinds is allowed,
but moving an existing failure from `LimitExceeded` to a new kind after 3.0
silently changes what callers match, so it has to happen before. Decide at
the same time whether `Error<E>` keeps `PartialEq` once it has context, or
compares only the kind. Cost: `FatFs` changes three return sites;
`hadris-fs` changes `NameError::kind` and the `std::io` mapping; tests that
match `LimitExceeded` for names or sizes change.

### F4. Value ranges of `NodeId` and `DirCursor` are unspecified (Must)

Evidence: `NodeId` is any `u64` (`node.rs:8`) and the root can have any id.
FUSE reserves 0 and fixes the root at 1, so the adapter swaps the root's id
with whatever the driver numbers 1 (`Adapter::ino`), and has no answer for a
driver that uses 0. `DirCursor` is any `u64` with 0 as the start
(`dir.rs:10`), but FUSE readdir offsets are `off_t`, and the adapter needs
two offsets for the `.` and `..` it synthesises, so it shifts cursors by 2
and fails with `EOVERFLOW` above `i64::MAX`. `FatFs` fits comfortably
(root 1, ids below `2^64`, cursors are slot numbers), and ISO, UDF and NTFS
ids fit as section 4.5 says, but the contract has to say so before drivers
exist that use the top bit.

Proposal: document that `NodeId::get()` is never 0, and that
`DirCursor::into_raw()` is below `2^63 - 16` for every cursor a driver
returns. Leave the root's value free; adapters map it. Cost: doc only.
`FatFs` already complies. A test in the contract kit (S6) can check both.

### F5. `remove` does not say what it expects to remove (Must)

Evidence: `remove(dir, name)` removes a file or an empty directory
(`driver.rs:96`). `unlink` must fail with `EISDIR` on a directory and
`rmdir` with `ENOTDIR` on a file, so the adapter looks the name up, reads
its metadata and checks emptiness before every removal (`Adapter::remove`).
The path helpers do the same (`remove_checked`, `api/paths.rs:172`). It costs
a lookup, a pin and a metadata call, and it is racy against a concurrent
change between the check and the removal (serialised in the prototype only
because it holds its own lock around both).

Proposal: `remove(dir, name, kind: RemoveKind)` with
`RemoveKind::{File, Dir, Any}` (non-exhaustive), or two methods
`remove_file` and `remove_dir`. The driver checks the type it already reads.
Cost: `FatFs::remove` compares `found.entry.is_dir()` with the kind; the
path helpers, the conformance adapter and the CLI pass a kind. Adding a
parameter later is a break, which is why this is Must.

### F6. How the traits grow under R10 (Must, docs and a test)

Evidence: new methods need a default and a forward in six places:
`FsDriver` and `FileSystem` themselves, `&mut F`, `Box<F>`, `&F` and
`AsDriver` (`driver.rs:283-531`), `Volume` (`api/volume.rs:94-212`),
`WithResolver`, and `impl_fs_driver!` (`macros.rs`). Two consequences are
not written down:

- `impl_fs_driver!` forwards a fixed list of write methods unless
  `read_only` is given (`macros.rs:76-115`). A new write method added to that
  list in 3.x would require every third-party format using the macro to have
  an inherent method of that name, which breaks them. New methods must be
  opt-in through `also = [..]`, like `parent` and `read_link`.
- Any user type that implements `FsDriver` or `FileSystem` by wrapping
  another (a logging layer, a jail) keeps the default body of a new method,
  so the new operation silently becomes `Unsupported` through the wrapper,
  while `capabilities()`, which it does forward, may still advertise it.
  E3's `DynFileSystem` has the same problem twice over, since it repeats the
  method list.

Everything else is extensible: all trait-facing structs have private fields
and `new` plus `with_*` constructors, the enums are non-exhaustive,
`RenameFlags` is a bitflags type, and `Capabilities` can gain flags.

Proposal: state both rules in R10, and add a hadris-fs test that calls every
trait method through each forwarding impl on a recording driver, so a
missed forward fails CI. Cost: docs and one test; no API change.

### F7. A listed id and a looked-up id can differ (Should)

Evidence: `read_dir_entry` returns `id_at(offset)`: the pinned id, else the
natural id, else `MOVED_IDS + slot` (`fatfs.rs:2185`). `lookup` pins through
`intern`, which uses a fallback id from `FALLBACK_IDS` when the natural id is
taken (`fatfs.rs:2192`). After a pinned file is renamed and a new file takes
its old slot, the two disagree: test
`readdir_and_lookup_can_disagree_on_the_inode_number`, and on the mount
`0x400000000000820D` from readdir against `0x8000000000000005` from `stat`.
`ls -i`, `find -inum`, and tools that match `d_ino` with `st_ino` see two
numbers for one file. The CLI's `find_name` (`app.rs:628`) only works because
it compares against a pinned id.

Proposal: add to the contract that, until the directory changes,
`DirEntry::node()` is the id a `lookup` of that name would return. In
`FatFs`, let `intern` take the moved-range id when the natural one is held
by a moved node, and use the fallback range only when that is taken too.
Cost: a few lines in `intern` and `natural_id`, plus one test.

### F8. No way to pin an id from a listing (3.x)

Evidence: `ls -f` on 3000 entries takes 10 ms, `ls -la` 1.8 s. The
difference is one `lookup` per entry, and a `FatFs` lookup scans the
directory from the start, so a full `ls -l` is quadratic. FUSE
`readdirplus` would avoid the lookups by returning attributes with the
entries, but each returned entry counts as a kernel lookup and must be
pinned, and the only way to pin is `lookup` by name. Kernel `getdents` plus
`stat` has the same shape.

Proposal: a `pin(node)` method, valid for ids just returned by
`read_dir_entry` (default `Unsupported`, with a `Capabilities` flag), so an
adapter can pin and read metadata without a second scan. `FatFs` can
implement it by interning the entry at the id's slot. Compatible in 3.x.

### F9. `Metadata` lacks fields that `stat` needs (3.x)

Evidence: `FileAttr` needs `blocks` (allocated 512-byte units); the adapter
guesses `len / 512`, so `du` is wrong for every cluster size above 512 and
for sparse or compressed NTFS files. There is no generation number: the
kernel compares generations to detect a reused node id
(`fuse_stale_inode`), and FAT ids are entry locations, which are reused
after a delete. There is no device number for nodes that `NewNode::Device`
can create, so ISO RRIP `PN` and cpio device entries cannot be read back.
`nlink` defaults to 1, including for directories, which `find` reads as
"unknown" (it passed), but the contract does not say so.

Proposal: add `allocated() -> Option<u64>`, `generation() -> u64` (0 when
unknown), and `device() -> Option<DeviceNumber>` with `with_*` setters, and
document that `nlink` 1 on a directory means unknown. `FatFs` can fill
`allocated` from the cluster count and `generation` from a per-mount counter
bumped when a table entry is created. All compatible, since `Metadata` has
private fields.

### F10. `forget` drops one pin (3.x)

Evidence: FUSE `forget(ino, nlookup)` and `batch_forget` release many pins
at once; the adapter loops, one driver call and one `Volume` lock round trip
per pin (`Adapter::forget`). After `drop_caches` the kernel sends thousands.

Proposal: `forget_n(node, count)` with a default that loops. Compatible.

### F11. `sync_node` is both "publish" and "durable" (Should)

Evidence: `FatFs` defers a written file's size and mtime until `sync_node`
or `sync` (`fatfs.rs:1285`), and `sync_node` also flushes the device
(`:1292`). FUSE calls `flush` on every `close(2)`, and the adapter must call
`sync_node` there to publish the size, so every close of a written file is a
device flush; with a real block device that is a cache flush per file in
`cp -r`. In the other direction, `fsync` is not durable today:
`impl BlockDevice for std::fs::File` flushes with `std::io::Write::flush`
(`crates/core/hadris-storage/src/sync.rs:95`), which does nothing for a file,
so neither `sync_node` nor `sync` reaches stable storage on a host image.

Proposal: document `sync_node` as "durable", and add a `publish_node` (or
`flush_node`) that writes pending metadata without flushing the device,
defaulting to `sync_node`; `File::close` and FUSE `flush` use it, FUSE
`fsync` uses `sync_node`. Separately, make the `File` device's `flush` call
`sync_data`. The new method is compatible; the `File` flush fix is a
behaviour change in `hadris-storage` worth making before 3.0.

### F12. Contract gaps that are one sentence each (Should)

| Gap | Evidence | Proposed wording |
|---|---|---|
| Access times | `FatFs::read_at` never updates atime; a write sets the access date (`fatfs.rs:1406`). The trait is silent. | Reads never change times; writes may. Callers that want atime call `set_metadata`. |
| Cancellation | A dropped `lookup` future in `async` could leave a pin. `FatFs` pins only after its last await, but the trait does not require it. | A cancelled call takes no pin and leaves no pending state. |
| Pending size vs mtime | `node_metadata` shows a pending size at once but the old mtime until `sync_node` (`FatFs::touch`, `fatfs.rs:1398`). | State which fields may lag until `sync_node`. |
| Root metadata | `FatFs` returns `Metadata::new(Dir)` for the root with no times (`fatfs.rs:803`), so the mount point shows 1970. | Allowed, but `FatFs` could use the volume label entry's times. |
| Local time | FAT stores local time; `DateTime` with no offset reads as UTC in the adapter. | Say that an absent offset means "UTC or unknown", and that FAT's clock decides. |
| `Busy` and hard links | See F1. | Only removing the last link of an open node is `Busy`. |

### F13. `DynFileSystem` does not need a second trait in sync (Should, design)

Evidence: E3's `DynFileSystem` repeats every `FileSystem` method with
`AnyError` (`dyn_fs.rs:24-60`), then implements `FileSystem` back for the
trait object. `tests/dyn_check.rs` shows that the sync `FileSystem` is
already dyn-compatible once its error is fixed:
`Box<dyn FileSystem<DeviceError = std::io::Error> + Send + Sync>` compiles,
and `lookup` and the default `resolve` work through it.

Proposal: in sync, `hadris-vfs` provides an `Erased<F>` wrapper that
implements `FileSystem<DeviceError = AnyError>` for any `F`, and stores
`Box<dyn FileSystem<DeviceError = AnyError> + Send + Sync>`. One method list
instead of two, so F6 has one fewer place to forget. The async forms still
need a boxed-future trait, since `async fn` is not dyn-compatible. Cost: a
design doc edit.

### F14. Missing operations for other formats (3.x)

All compatible under R10, each with a default and a `Capabilities` flag:

| Operation | Needed by | Note |
|---|---|---|
| `link(node, dir, name)` | NTFS and UDF write, FUSE `link` | `Capabilities::hard_links` exists but nothing creates a link. `Tree::add_hard_link` covers writers only. |
| `NewNode::Fifo`, `NewNode::Socket` | cpio, ISO RRIP, FUSE `mknod` | `FileType` has both; `NewNode` (non-exhaustive) does not. |
| `RenameFlags::EXCHANGE` | FUSE `RENAME_EXCHANGE` | `FatFs` already rejects unknown flags with `Unsupported`. |
| Extended attributes and named streams | NTFS ADS, UDF named streams, macOS clients | Section 5.5 keeps NTFS streams native, which is enough for 3.0. |
| Canonical name of a node | CLI `find_name` (`app.rs:628`), case-insensitive formats | A `lookup` that also writes the stored name into a `NameBuf`. |
| `FsStats` available space and free inodes | FUSE `statfs` `bavail`, `ffree` | Reserved blocks on NTFS; FAT12/16 root entry limit. |
| Time resolution per field | FAT: created 10 ms, modified 2 s, accessed 1 day | `Capabilities::timestamp_resolution_ns` covers only modified. |
| `DateTime` from and to `SystemTime` | Every std adapter | The adapter writes both conversions by hand. |

### F15. What already fits

These were checked and need nothing:

- **Stable ids across rename.** A pinned node keeps its id through `rename`
  (test `rename_replaces_a_looked_up_target`), which is what FUSE needs.
- **Resumable readdir.** `DirCursor` resumes across calls from any offset
  the kernel hands back, pages of 7 entries reassemble 300 entries exactly,
  and concurrent creation and deletion between pages never duplicates or
  drops an unchanged entry (tests `readdir_pages_resume_from_offsets`,
  `readdir_survives_changes_between_pages`, mount scenario
  `readdir_while_creating`). `FatFs` re-walks the cluster chain on each call
  (`Walk::new` in `read_dir_entry`, `fatfs.rs:822`), but `ls -f` of 3000
  entries still takes 10 ms, and the cursor is opaque, so `FatFs` can later
  encode the cluster in it without a contract change.
- **Rename flags.** `NO_REPLACE` maps to `RENAME_NOREPLACE`, and unknown
  flags are refused.
- **Ignored mode, uid and gid.** `cp -p` and `tar` succeed, as the
  `set_metadata` contract intends.
- **Unmount.** `fuser` drops the adapter when the session ends, `destroy`
  calls `sync`, and pending sizes held by the driver's own pins reach the
  disk even after a lazy unmount with a file still open.
- **Sharing.** `Volume` with `StdMutex` behind four FUSE threads, and
  `forget` from any thread, worked without deadlock, and `fsck.fat`
  found nothing afterwards.
- **Read-only formats.** A read-only ISO or UDF view leaves the write
  methods at their `ReadOnly` defaults, reports `writable` false, and the
  adapter answers `EROFS` before touching anything.
- **exFAT.** It shares the node table and FAT's identity model, so F1, F7
  and F9 apply unchanged and nothing else is new.

### F16. Other findings

- **R9.** `MountOptions::with_read_only(bool)` (`hadris-fat`,
  `options.rs:66`) is a bool parameter. `with_read_only()` without an
  argument, or a two-variant enum, before 3.0.
- **The test driver does not enforce the contract.** `MemFs` never returns
  `Busy`, ignores `NO_REPLACE`, and lets `rename` replace a non-empty
  directory (`mem_fs.rs`, `remove` and `rename`). Nothing in `hadris-fs`
  checks that a driver follows the contract; the conformance suite checks
  FAT semantics through the node API but not the pin, cursor and error
  rules. A generic contract kit over `FileSystem` in `hadris-tests`, seeded
  with the sequences in `tests/ops.rs`, should exist before ISO ports.
- **Async parity.** The prototype is sync only; `fuser`'s async API is
  experimental. The parity report (`scripts/check-v3-api.py parity`) shows
  only the documented differences for `hadris-fs`: host helpers, the
  `Iterator` and `std::io` impls, and lock kinds (`Spin` and `StdMutex` in
  sync, `AsyncMutex` and the `embassy-sync` `Local` in async). Nothing in
  the trait shapes blocks an `async_send` FUSE adapter: `forget`,
  `capabilities` and `root` are sync and never block.
- **The conformance adapter** reopens the volume for every operation and
  tracks its own pins in a `Vec` (`generic.rs:70-77`); it reads the label
  through `vol.lock()` (`generic.rs:279`), as section 4.3 intends. A pin
  guard type would help it and the CLI, and can be added any time.

## 3. Proposed changes, in priority order

| # | Change | Severity | Cost to `FatFs` and tests |
|---|---|---|---|
| 1 | Split lookup pins from opens: removing a pinned node succeeds, only an open node's last link is `Busy`; add `open`/`close` (F1) | Must | Node state gains `opens` and `unlinked`; `remove` and `replace_entry` change their check; handle tests and Busy tests change |
| 2 | `remove(dir, name, RemoveKind)` (F5) | Must | One comparison in `FatFs::remove`; path helpers, conformance adapter and CLI pass a kind |
| 3 | `NameTooLong` and `FileTooLarge` kinds; decide `Error<E>` context and equality (F3) | Must | Three return sites; `NameError::kind`; `std::io` mapping; tests matching `LimitExceeded` |
| 4 | Keep `Metadata` `Copy`, drop `extra()` from the design (F2) | Must decide | Doc only |
| 5 | Reserve `NodeId` 0; bound `DirCursor` below `2^63 - 16` (F4) | Must | Doc only; `FatFs` complies |
| 6 | Write down R10 growth rules for `impl_fs_driver!` and wrappers; forwarding test (F6) | Must | Docs and one hadris-fs test |
| 7 | Listed id equals looked-up id; fix `FatFs::intern` (F7) | Should | A few lines and one test |
| 8 | `publish_node` beside a durable `sync_node`; `File` device flush calls `sync_data` (F11) | Should | New default method; one-line storage fix |
| 9 | One-sentence contract additions: atime, cancellation, pending fields, root times, local time (F12) | Should | Docs; optional root times in `FatFs` |
| 10 | Sync `hadris-vfs` erases through `dyn FileSystem<DeviceError = AnyError>` (F13) | Should | Design doc |
| 11 | Contract kit over `FileSystem` in `hadris-tests`; make `MemFs` follow the contract (F16) | Should | New tests; `MemFs` fixes |
| 12 | `with_read_only()` without a bool (F16, R9) | Should | `hadris-fat` options, CLI and tests |
| 13 | `pin(node)` for listed ids, `forget_n`, `Metadata::{allocated, generation, device}`, `link`, `NewNode::{Fifo, Socket}`, `RenameFlags::EXCHANGE`, `FsStats` extras, per-field time resolution, `SystemTime` conversions (F8 to F10, F14) | 3.x | Each compatible under R10 |

## 4. Open questions for the maintainer

- F1 changes the answer to Q3. Is "removing a pinned but unopened node
  succeeds, and its id then answers `NotFound`" acceptable, or should the
  trait go further and support POSIX unlink of open files through orphan
  tracking?
- F3: new kinds, or a public `Detail` next to the kind? New kinds are
  simpler for errno and `std::io` mapping; `Detail` keeps the kind set small
  as section 4.6 wants.
- F5: one method with `RemoveKind`, or `remove_file` plus `remove_dir`?
- F11: should `File::close` publish only, or stay durable as it is now?

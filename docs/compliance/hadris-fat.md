# `hadris-fat` compliance profile

The crate is reviewed against Microsoft's FAT 1.03 document for FAT12,
FAT16, and FAT32 and the Microsoft exFAT 1.00 specification. Atomic mappings
and executable evidence are recorded in
[`spec/requirements/hadris-fat.json`](../../spec/requirements/hadris-fat.json).

This pass fixed FAT32 writes that discarded the reserved high nibble, rejected
clusters larger than 32 KiB and unknown FAT32 filesystem versions, and made
normal exFAT mounting enforce boot-region, up-case-table, and file entry-set
checksums.

Known gaps remain explicit in the catalog. FAT32 mounts do not honor the
active-FAT selection when mirroring is disabled and do not recover through the
backup boot record. exFAT mounts validate only the main boot region, and
fragmented up-case tables are rejected rather than followed through the FAT.

The FAT12/16/32 conformance suite uses a specification-oriented raw image
oracle rather than another filesystem tool as its ground truth. mtools,
dosfstools, macOS FAT tools, native Linux/macOS kernel mounts, and the Rust
`fatfs` crate are measured independently and may disagree or crash without
changing the expected FAT semantics.

## Interoperability results

The specification suite applies one curated trace, 17 focused edge-case
traces, and 16 deterministic generated traces to each of FAT12, FAT16, and
FAT32, then 29 rejection scenarios whose final operation must fail without
touching the image, and three limit exercises sized from the formatted
geometry. Peer scoring uses the curated, focused, rejection, and limit
scenarios so a peer failure cannot mask the rest of a 64-operation generated
trace. The harness compares displayed paths, entry kinds, file contents,
stable attributes, and volume labels. Timestamps are excluded. The raw oracle
additionally validates geometry, mirrored FATs, reserved entries, the FAT32
FSInfo free count against the FAT, cluster chains, cross-links, directory
records, dot-entry placement, LFN sequences and checksums, the 8.3 alias
character set, and unique short aliases.

### Consumers of Hadris images

| Consumer | FAT12 | FAT16 | FAT32 | Interpretation |
|----------|-------|-------|-------|----------------|
| Hadris specification oracle | 21/21 | 21/21 | 21/21 | All 18 peer scenarios and three geometry-sized limit exercises match the model. |
| mtools 4.0.49 reader | 14/16 | 14/16 | 14/16 | The two failures are names outside the Basic Multilingual Plane, which mtools transliterates instead of reading as surrogate pairs. |
| dosfstools `fsck.fat` 4.2 | 16/16 | 16/16 | 16/16 | Every oracle-valid Hadris image passed a read-only structural check. |
| Rust `fatfs` master (`2aefc2a`) reader | 16/16 | 16/16 | 16/16 | The independent Rust reader reconstructed every oracle-valid tree, surrogate pairs included. |
| macOS `fsck_msdos` | 2/2 | 2/2 | 2/2 | The curated trace and one deterministic trace passed read-only checks on macOS 26.6.2. |

The per-width oracle count covers the 18 peer scenarios and three limit
exercises. The peer rows retain their last recorded 16-scenario qualification
and use a different denominator. These are bounded interoperability results,
not claims that the tools accept every possible image Hadris can produce.
Native writable-mount tests for the Linux `vfat` and macOS FAT drivers are also
available in the harness, but are reported separately because they require
host mount privileges.

### Resolved in 2.3.0

The 2.3.0 suite now gates the following cases, which failed in 2.2.0:

- A rename that only changed the case of an 8.3 name (`lower.txt` to
  `LOWER.TXT`) was refused as an existing entry. Case-only renames now
  succeed.
- Long-name lookups were case-sensitive, so `MIXED CASE NAME.BIN` was created
  next to `Mixed Case Name.bin`, a directory could be created next to another
  differing only in case, and a rename onto such a name succeeded. Lookups
  are now case-insensitive.
- Moving a directory into its own subtree (`/A` to `/A/B/A` or `/A/A`) was
  accepted and detached the subtree into an unreachable cycle. It is now
  rejected.
- The reserved characters `: * ? " < > |` and control characters were
  accepted in long names. They are now rejected.
- `..dots` received the alias `        DOT`, which started with a space and
  which `fsck.fat` reported as a bad short name. Leading-dot names now get a
  valid alias.
- When the data region filled, the aborted write leaked the clusters it had
  already allocated on FAT12 and FAT16, and left the FAT32 FSInfo free count
  stale. Failed writes now release their clusters and persist a valid FSInfo.

### Peer writers

| Producer and operations | Completed | Oracle-valid | Additional result |
|-------------------------|-----------|--------------|-------------------|
| Hadris | 54/54 traces, 87/87 rejections, 9/9 limits | 54/54 completed traces, 9/9 limits | The hosted 2.3.0 suite gates every result. |
| `mkfs.fat` + mtools 4.0.49 | 44/54 traces, 63/87 rejections, 8/9 limits | 42/44 completed traces, 8/9 limits | Hadris read and `fsck.fat` accepted all completed images. |
| Rust `fatfs` master (`2aefc2a`) | 39/54 traces, 60/87 rejections, 2/9 limits | 30/39 completed traces, 2/9 limits | Hadris read every oracle-valid image. |

The aggregate bidirectional semantic scores are 155/198 (78.28%) for mtools
and 140/198 (70.71%) for Rust `fatfs` master. An aborted command is counted as
a failed writer attempt, while a peer mismatch never changes the Hadris ground
truth.

mtools: three traces abort inside `mren` (below); `mren` refuses a case-only
rename; names outside the Basic Multilingual Plane are transliterated, so
`mcopy` cannot address a directory it just created under such a name and a
255-unit name whose first character is a surrogate pair is rejected as too
long; and moving a directory under a FAT32 parent whose cluster number is
above 65535 writes only the low 16 bits into the `..` entry (`37690` for
parent `103226`). Accepted rejections are mostly `cp`-like semantics rather
than defects: copying onto an existing directory or `.` places the file
inside it, and `mren` onto an existing directory moves the source into it.
mtools also creates case-insensitive duplicates.

Rust `fatfs`: the leading-multibyte-character panic
([issue #118](https://github.com/rafalh/rust-fatfs/issues/118)) now also fires
for `σigma.txt` and a 255-unit name beginning with an emoji; names outside the
BMP are rejected as unsupported characters; moved directories keep a stale
`..` ([issue #119](https://github.com/rafalh/rust-fatfs/issues/119)); a
case-only rename is silently ignored; case-insensitive duplicates, moves into
a directory's own subtree, and `.`/`..` file names are accepted. At the
limits, a long-name entry is written for every 8.3 name
([issue #17](https://github.com/rafalh/rust-fatfs/issues/17)), so a FAT12/16
root holds 255 files and the 256th fails with `Write zero`; filling a FAT16
volume leaves orphaned long-name entries in the root; and filling a FAT32
volume leaves the FSInfo next-free hint outside the cluster heap.

### Focused edge cases

The focused traces are deliberately short and each targets a separate
failure mechanism:

- expanding a short directory name into an LFN entry set after deleting the
  following sibling;
- filling a 512-byte subdirectory entry cluster, reusing adjacent deleted
  slots, and extending the directory;
- generating unique aliases for six long names that collapse to the same
  short-name base, and for basenames shorter than the six characters that
  precede a numeric tail;
- moving a nested directory between two parents and then back to the root;
- truncating a multi-cluster file across exact and off-by-one boundaries,
  freeing it to zero, and allocating it again;
- writing names at and immediately above 13-UTF-16-unit LFN slot boundaries,
  up to the 255-unit maximum, including names beginning with a multibyte
  UTF-8 character or a surrogate pair;
- names that fit 8.3 apart from case, and renames that change only case;
- twelve long names sharing one alias prefix with an explicit `~1` name
  already present, then churn on the freed tails;
- deleting alternate entries in a subdirectory so later names need runs
  wider than any hole, including a 20-slot name;
- twelve levels of long directory names with moves of the top and the
  deepest directory;
- hidden and system directories and read-only files through renames, moves,
  and attribute changes;
- appends and truncations landing exactly on multiples of every cluster size
  the suite formats with; and
- reusing clusters freed from a directory full of entries and from a file
  whose contents look like directory entries.

The limit exercises fill a fixed FAT12/16 root directory to its last slot and
then try an overflow, a long name that does not fit the single free slot,
and a final 8.3 name; write files until the data region is exhausted, check
that every cluster can be allocated and that deleted space is reusable; and
write one extent covering most of the volume so its chain crosses every FAT
sector boundary and, on FAT32, pushes later allocations above cluster 65535.

These cases were informed by the mtools directory-cache failure, the mtools
and dosfstools change logs, and Rust `fatfs` regressions including
[#17](https://github.com/rafalh/rust-fatfs/issues/17),
[#42](https://github.com/rafalh/rust-fatfs/issues/42),
[#118](https://github.com/rafalh/rust-fatfs/issues/118), and
[#119](https://github.com/rafalh/rust-fatfs/issues/119). The hosted suite runs
the Hadris/oracle traces and limit exercises in about seven seconds;
tool-dependent reports remain manual.

### Why the mtools rename is a valid test

The three mtools failures are the same operation on FAT12, FAT16, and FAT32:

```text
mren ::/D000 "::/Renamed Directory 0009"
Assertion failed: (entry->beginSlot == beginSlot), function freeDirCacheRange,
file dirCache.c, line 198.
```

The source is a valid, non-empty directory in the root. Immediately before the
rename, dosfstools `fsck.fat -n` accepts the image. Renaming that directory to
another 8.3 name succeeds; changing only the destination to the long name above
triggers the assertion. The documented same-directory `mren` form with a bare
destination name triggers the same assertion.

The [Microsoft FAT specification v1.03](https://download.microsoft.com/download/0/8/4/084c452b-b772-4fe5-89bb-a0cbf082286a/fatgen103.doc),
“Name Limits and Character Sets” under “Long Directory Entries” on page 29,
allows long names up to 255 characters and permits embedded spaces. `Renamed
Directory 0009` is 22 characters and contains only letters, digits, and
embedded spaces. The long-directory-entry structure on pages 26–28 applies to
the name associated with the following short entry; the short entry retains
the `ATTR_DIRECTORY` bit, with no restriction that the directory be empty.
The [GNU mtools `mren` documentation](https://www.gnu.org/software/mtools/manual/html_node/mren.html)
also explicitly supports renaming subdirectories, and its
[VFAT long-name documentation](https://www.gnu.org/software/mtools/manual/html_node/long-names.html)
describes generating a companion short alias for names that do not fit 8.3.
The failure is therefore recorded as an mtools writer crash rather than an
invalid generated operation or a rejection of a Hadris image.

The exact reports are regenerated under `tests/target/reports/fat/` by the manual
commands documented in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#fat-conformance).

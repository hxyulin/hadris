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

The manual FAT12/16/32 conformance suite uses a specification-oriented raw
image oracle rather than another filesystem tool as its ground truth. mtools,
dosfstools, macOS FAT tools, native Linux/macOS kernel mounts, and the Rust
`fatfs` crate are measured independently and may disagree or crash without
changing the expected FAT semantics.

## Interoperability results

The suite applies one curated trace and 16 deterministic generated traces to
each of FAT12, FAT16, and FAT32. It compares displayed paths, entry kinds, file
contents, stable attributes, and volume labels. Timestamps are excluded. The
raw oracle additionally validates geometry, mirrored FATs, reserved entries,
FAT32 metadata, cluster chains, cross-links, directory records, LFN sequences
and checksums, and unique short aliases.

### Consumers of Hadris images

| Consumer | FAT12 | FAT16 | FAT32 | Interpretation |
|----------|-------|-------|-------|----------------|
| Hadris specification oracle | 17/17 | 17/17 | 17/17 | Every image was structurally valid and matched the semantic model. |
| mtools 4.0.49 reader | 17/17 | 17/17 | 17/17 | `mdir`, `mtype`, and related commands reconstructed the expected tree. |
| dosfstools `fsck.fat` 4.2 | 17/17 | 17/17 | 17/17 | Every completed Hadris image passed a read-only structural check. |
| Rust `fatfs` 0.3.6 reader | 17/17 | 17/17 | 17/17 | The independent Rust reader reconstructed the expected tree. |
| macOS `fsck_msdos` | 2/2 | 2/2 | 2/2 | The curated trace and one deterministic trace passed read-only checks on macOS 26.6.2. |

These are bounded interoperability results, not claims that the tools accept
every possible image Hadris can produce. Native writable-mount tests for the
Linux `vfat` and macOS FAT drivers are also available in the harness, but are
reported separately because they require host mount privileges.

### Peer writers

| Producer and operations | Completed | Oracle-valid | Additional result |
|-------------------------|-----------|--------------|-------------------|
| Hadris | 51/51 | 51/51 | mtools and `fsck.fat` accepted all 51 images. |
| `mkfs.fat` + mtools 4.0.49 | 48/51 | 48/48 completed traces | Hadris read all 48 completed images and `fsck.fat` accepted all 48. Three traces aborted inside `mren`. |
| Rust `fatfs` 0.3.6 | 51/51 | 0/51 | Every trace produced LFN records before the reserved `.` and `..` short entries; see [rust-fatfs issue #117](https://github.com/rafalh/rust-fatfs/issues/117). |

The aggregate bidirectional semantic scores are 99/102 (97.06%) for mtools
and 51/102 (50.00%) for Rust `fatfs` 0.3.6. An aborted command is counted as a
failed writer attempt, while a peer mismatch never changes the Hadris ground
truth.

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

The exact reports are regenerated under `target/fat-conformance/` by the manual
commands documented in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#fat-conformance).

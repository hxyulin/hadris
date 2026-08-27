# `hadris-iso` compliance profile

The tracked requirement catalog is
[`spec/requirements/hadris-iso.json`](../../spec/requirements/hadris-iso.json).
It targets the ECMA-119:1987 primary hierarchy, a 2,048-byte logical-sector
creation profile, and the crate's allocating and allocation-free readers.

The crate is not claimed to implement all of ECMA-119. In particular, volume
partitions, extended-attribute record bodies, interleaved files, multi-volume
sets, and record-structured files are outside the current creation profile.
Optional namespaces and boot extensions have their own source documents and
must not be treated as evidence for the base ECMA-119 profile.

The audit fixed several cases where self-round-trip tests had hidden invalid
output or silent misreads: corrupt descriptor terminators are rejected,
redundant byte-order copies in directory records are checked, allocating reads
account for extended-attribute blocks, interleaved input is rejected rather
than read contiguously, directory records are kept within sector boundaries,
directory identifiers no longer receive file version suffixes, path-table
siblings are sorted, and descriptor dates use fixed-width digits.

Follow-up regressions now also reject inconsistent redundant PVD integers and
a descriptor sequence without a primary descriptor. Raw-image tests directly
prove directory-sector padding, directory identifier grammar, and adversarial
path-table ordering.

Remaining `partial` entries are intentional work items. A status becomes
`verified` only when a clause-sized claim has direct executable evidence; a
successful Hadris-to-Hadris round trip is not by itself a conformance oracle.

## Conformance methodology

The library test suite contains a raw ECMA-119 oracle that does not use the
Hadris parser. It validates the descriptor sequence, redundant endian copies,
declared volume bounds, matching Type-L and Type-M path tables, path-table
ordering, recursive directory records, sector padding, Level 1 identifiers,
file extents, and file contents. The expected semantic tree is then read
independently through Hadris. External tools and operating-system readers are
peers measured against this ground truth.

The portable fixture includes empty and multi-sector files, nested directories,
and a directory large enough to span multiple logical sectors. The normal
in-process test is fast; external producers and native mounts are explicit
manual tests that can run on an actual Linux, macOS, or Windows host.

## Consumers of Hadris images

The following results were measured on 2026-08-27. “Oracle” means that the
peer-produced primary hierarchy passed the independent ECMA-119 checks;
“semantic” means names, entry kinds, and exact file contents matched.

| Consumer | Semantic result | Interpretation |
|---|---:|---|
| Raw ECMA-119 oracle | 2/2 | Both Hadris scenarios were structurally valid and matched the semantic model. |
| Hadris reader | 2/2 | Both writer images reconstructed the expected tree and exact contents. |
| xorriso/libisofs 1.5.8 extractor | 2/2 | Both Hadris images reconstructed the expected host tree. |
| macOS 26.6.2 built-in ISO reader | 2/2 | Both Hadris images mounted and matched the semantic model. |
| Linux kernel ISO driver | 2/2 | Both Hadris images mounted read-only and matched the semantic model. |
| Windows `Mount-DiskImage` | Not measured | The portable adapter is available for a manual run on Windows. |

Linux is mounted with `map=off` to preserve identifier case. Numeric file
version suffixes exposed by a native mount are removed from comparison paths,
as the semantic model represents `README.TXT;1` as `README.TXT`.

## Peer producers

| Producer | Completed | Oracle-valid | Hadris read |
|---|---:|---:|---:|
| Hadris | 2/2 | 2/2 | 2/2 |
| xorriso/libisofs 1.5.8 | 2/2 | 2/2 | 2/2 |
| cdrtools mkisofs 3.02a09 | 2/2 | 2/2 | 2/2 |
| macOS 26.6.2 `hdiutil makehybrid` | 2/2 | 0/2 | 2/2 |

xorriso/libisofs has an aggregate bidirectional semantic score of 4/4
(100.00%). cdrtools `mkisofs` is measured only as a producer and scores 2/2
(100.00%). Peer deviations do not change the Hadris ground truth.

The macOS producer omits the file-version component from primary-hierarchy file
identifiers. ECMA-119:1987 clause 7.5.1 requires Separator 2 (`;`) followed by
a version number from 1 through 32,767. Hadris deliberately accepts the
versionless names for interoperability, but the peer report records the
standards deviation. No external implementation is used as an authority for
Hadris output.

xorriso is the relevant libburnia command-line peer for ISO image creation.
Its filesystem generation is provided by libisofs; libburn provides optical
drive and media access, so testing libburn directly would not add an
independent ISO filesystem producer.

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

Remaining `partial` entries are intentional work items. A status becomes
`verified` only when a clause-sized claim has direct executable evidence; a
successful Hadris-to-Hadris round trip is not by itself a conformance oracle.

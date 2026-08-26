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

# `hadris-udf` compliance profile

The atomic catalog in
[`spec/requirements/hadris-udf.json`](../../spec/requirements/hadris-udf.json)
keeps ECMA-167 base-format requirements separate from the UDF 1.02 restrictions
published in ECMA TR/112-7.

This distinction matters because the pinned ECMA-167 document is the third
edition while UDF 1.02 was originally based on an earlier edition. Only stable
base structures are attributed to ECMA-167:1997; UDF-profile restrictions are
attributed directly to the current ECMA technical report.

The writer records exactly two permitted anchor locations, declares
sixteen-sector main and reserve descriptor-sequence extents, a single closed
integrity descriptor and a single file set, and begins every directory with
its parent identifier. Raw-image regressions cover these requirements, and
`udfinfo`, `mkudffs`, 7-Zip, the Linux kernel and macOS read the output.

The reader selects the prevailing volume descriptors, falls back to the
reserve sequence and the backup anchors, and follows allocation extent
descriptors and every allocation descriptor form. It does not interpret
implementation use or unallocated space descriptors, indirect entries or
stream directories, and refuses virtual, sparable and metadata partitions,
so coverage of Part 3 and Part 4 stays partial.

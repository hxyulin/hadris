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

## Bridge images

The bridge writer (`plan_bridge` and `write_bridge`, formerly `hadris-cd`)
follows ECMA TR/71's UDF Bridge logical-sector image, cataloged beside the
UDF requirements. This is not evidence for raw CD sector framing: no pinned
ECMA-130, Yellow Book, or equivalent recording-layer source is available, so
the catalog makes no claim about raw-sector sync, headers, modes, EDC, ECC,
subchannels, or physical recording, nor about El Torito, Joliet, Rock
Ridge/SUSP, UEFI partitioning, or later UDF bridge profiles in a bridge.

Raw-image regressions cover the pinned profile: fixed 2,048-byte sectors,
consecutive recognition descriptors, permitted anchors, sixteen-sector main
and reserve descriptor sequences, a single closed integrity descriptor, one
partition, and short allocation descriptors.

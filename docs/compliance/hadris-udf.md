# `hadris-udf` compliance profile

The atomic catalog in
[`spec/requirements/hadris-udf.json`](../../spec/requirements/hadris-udf.json)
keeps ECMA-167 base-format requirements separate from the UDF 1.02 restrictions
published in ECMA TR/112-7.

This distinction matters because the pinned ECMA-167 document is the third
edition while UDF 1.02 was originally based on an earlier edition. Only stable
base structures are attributed to ECMA-167:1997; UDF-profile restrictions are
attributed directly to the current ECMA technical report.

The writer records exactly two permitted anchor locations and declares
sixteen-sector main and reserve descriptor-sequence extents. Raw-image
regressions cover both requirements. Reader coverage remains partial for
prevailing descriptor selection, allocation-extent chaining, and several
mandatory descriptor families.

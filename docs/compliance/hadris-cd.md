# `hadris-cd` compliance scope

The available authoritative profile is ECMA TR/71's UDF Bridge logical-sector
image, cataloged in
[`spec/requirements/hadris-cd.json`](../../spec/requirements/hadris-cd.json).
Despite the crate name, this is not evidence for raw CD sector framing.

No pinned ECMA-130, Yellow Book, or equivalent recording-layer source is
available. Consequently the catalog makes no claim about raw-sector sync,
headers, modes, EDC, ECC, subchannels, or physical recording. The available
report also cannot substantiate El Torito, Joliet, Rock Ridge/SUSP, UEFI
partitioning, or later UDF bridge profiles.

The logical-sector writer now has direct raw-image evidence for the pinned
profile: fixed 2,048-byte sectors, consecutive recognition descriptors,
permitted anchors, sixteen-sector main and reserve descriptor sequences, a
single closed integrity descriptor, one partition, and short allocation
descriptors. Higher recording-layer and extension claims remain out of scope.

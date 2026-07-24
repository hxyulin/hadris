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

The extraction identified a concrete profile gap: main and reserve UDF volume
descriptor sequence extents are currently declared as six sectors, while
ECMA TR/71 requires each extent to occupy at least sixteen logical sectors.

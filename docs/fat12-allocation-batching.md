# FAT12 allocation batching

## Prerequisite: interrupted split-entry recovery

FAT12 entries occupy twelve bits in a two-byte window. When that window crosses
an underlying device-block boundary, writing it requires two transfers. Before
this change, interruption after the first transfer could leave a partial link:
for example, allocating cluster 341 as end-of-chain could temporarily encode
cluster 15. Mirroring that partial value and reclaiming the held allocation
could follow an unrelated live file's chain.

The raw FAT state now records the intended value and original allocation status
while writing a split entry in the active FAT. `mirror` completes that entry
before copying secondary FATs. The record survives failed or cancelled repairs,
clears only after the active entry completes, and preserves exactly-once
free-cluster accounting. Recovery is also tracked on volumes with one FAT copy.
This is recovery within the running driver; it does not add a persistent journal
or guarantee recovery following process termination or power loss.

The focused regression fails against the original code with a partial link to
cluster 15. Its independent packed-entry reader checks every data-cluster entry
in every FAT copy, including the neighbors sharing nibbles and unrelated live
clusters 15/255. It exercises allocation, linking and freeing on 512/4096-byte
sector/device-block combinations with one and two FAT copies, injects each
write failure, and cancels both the update and its repair at every await.

Existing hosted async future caps pass unchanged. All three firmware targets
pass the repository's flash/state/stack budgets. The split-entry record adds
16 bytes to the embedded driver state: the bare-metal four-slot FAT state is
944 bytes. The thumbv7em FAT logger occupies 40,576 bytes, below its 44 KiB
regression ceiling; the existing 20 KiB target remains tracked separately.

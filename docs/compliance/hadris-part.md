# `hadris-part` compliance scope

The local audit could not establish GPT or generic MBR conformance: the pinned
UEFI 2.11 download remains unresolved, and no authoritative MBR layout source
is pinned. Existing broad `full` annotations were therefore downgraded.

The Microsoft exFAT 1.00 document provides only narrow identifier requirements
relevant to partitioning; those must not be interpreted as evidence for the
rest of GPT or MBR. Independently found robustness fixes, such as complete
4 KiB header padding and correct mixed-endian UUID version bits, remain useful
but are not given clause-level conformance status without the source.

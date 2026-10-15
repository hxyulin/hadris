# `hadris-part` compliance scope

The local audit could not establish GPT or generic MBR conformance: the pinned
UEFI 2.11 download remains unresolved, and no authoritative MBR layout source
is pinned. Existing broad `full` annotations were therefore downgraded.

The Microsoft exFAT 1.00 document provides only narrow identifier requirements
relevant to partitioning; those must not be interpreted as evidence for the
rest of GPT or MBR. Independently found robustness fixes, such as complete
4 KiB header padding and correct mixed-endian UUID version bits, remain useful
but are not given clause-level conformance status without the source.

Interoperability evidence comes from GPT fdisk instead. The conformance
suite compares Hadris edits with `sgdisk` byte for byte: moving the backup
GPT to the end of a grown disk (`Gpt::relocate_backup`, as `sgdisk -e`),
resizing the entry arrays (`Gpt::set_entry_count`, as
`sgdisk --resize-table`) and round-tripping hybrid MBRs that `sgdisk -h`
writes, and `sgdisk -v` checks the tables Hadris writes. MBR reading
follows extended boot record chains up to 256 logical partitions, and the
`part_read` fuzz target edits, writes and re-reads what it parses. This is
behavioral agreement with one implementation, not conformance to UEFI.

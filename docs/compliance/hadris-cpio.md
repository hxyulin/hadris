# `hadris-cpio` compliance profile

The crate implements a streaming reader for uncompressed `newc` (`070701`),
CRC-newc (`070702`), `odc` (`070707`) and old binary archives, and a
streaming writer for `newc`, CRC-newc and `odc`. The atomic evidence is in
[`spec/requirements/hadris-cpio.json`](../../spec/requirements/hadris-cpio.json).

This is narrower than the Linux initramfs buffer grammar. The reader follows
concatenated uncompressed archives and the zero padding between them when
the caller asks (`continue_after_trailer`), but it does not decompress
compressed members. GNU cpio behavior is recorded only as informative
interoperability context, not as a normative wire-format specification.

Checksums are verified whether an entry's data is read or skipped. Filename
termination, zero padding, trailer size, the `070701` check field and the
mode's file type bits are validated, and an archive may end at an aligned
entry boundary without a trailer unless `ReaderOptions::with_strict_trailer`
is set. The writer checks every field width before it writes an entry,
rejects empty symbolic-link targets and the trailer's name, and writes hard
link groups as GNU cpio does: one inode, the complete link count on every
name, and the data on the last name. The reader returns each name with its
inode and link count; reconstructing the links is left to the caller, as
the CLI's `extract` does.

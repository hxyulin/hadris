# `hadris-cpio` compliance profile

The crate implements a streaming reader and writer for one uncompressed
`newc` (`070701`) or CRC-newc (`070702`) archive. The atomic evidence is in
[`spec/requirements/hadris-cpio.json`](../../spec/requirements/hadris-cpio.json).

This is deliberately narrower than the Linux initramfs buffer grammar. Hadris
does not currently compose or scan arbitrary sequences of zero padding,
compressed members, and multiple archives. GNU cpio behavior is recorded only
as informative interoperability context, not as a normative wire-format
specification.

The compliance pass added mandatory checksum verification to both read and
skip paths, validates filename termination and zero padding, enforces checked
name/data lengths, rejects empty symbolic-link targets, validates trailer
size, and emits a consistent link count for hard-link groups. Remaining gaps
include trailerless aligned archives, reader-side hard-link reconstruction,
host hard-link discovery, and full initramfs buffer composition.

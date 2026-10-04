# Hosted FAT rename cancellation recovery

A cancelled rename could publish the destination short entry and leave the
source entry owning the same clusters. `sync()` previously lacked the source
cleanup state, so the raw oracle reported a cross-link even after recovery.
The minimal FAT32 case renames nonempty `OLD.TXT` to
`A replacement long name.txt`, cancels at await budget five, then syncs.

The node driver now retains one bounded rename record before changing directory
entries. The record owns the source and destination runs, the exact expected
destination short entry, parent-directory changes, pinned identities and, for
replacement, a snapshot of the old target's at most 21 slots.

## Publication and recovery

Before insertion, the destination short slot is explicitly marked free. This
prevents stale bytes behind a directory's logical End from looking like a
newly published entry. Publication is armed only when the final destination
short-entry encoder runs, after the long-name/end-marker phase. Recovery
requires both this phase and a visible short entry matching all expected bytes.
An untouched old replacement target cannot establish publication.

Before publication, recovery clears the incomplete destination and restores a
saved replacement target. It restores the original directory End marker when
the destination consumed it. Once rollback is selected, that decision persists
before any cleanup await, so cancellation while restoring a byte-identical
empty target cannot turn rollback into a committed move on retry.

After publication, recovery finishes the directory's `..` update, removes the
entire source run, invalidates a replaced target's pinned identity and redirects
the source's pinned identity to its new slot. It retains the original source
ID and any dirty metadata. When insertion consumes End, it also places the new
End after the destination so formerly ignored trailing garbage stays hidden.

The rename record remains present through every recovery await. Generic FAT
mirroring runs first; generic orphan-run cleanup and displaced-target chain
reclamation run only after the rename resolves. The existing held-chain state
advances during reclamation, allowing interrupted target freeing to resume
without restoring its old head or freeing the chain twice. Errors after
publication may finish the committed move rather than restore the old target.

This recovery applies while the same mounted driver remains alive. It is
in-memory state, not a persistent journal or a guarantee of atomic rename
across a process crash, power loss, or dropping the driver without syncing.

## Bounds and compatibility

There is at most one boxed rename record and one bounded replacement snapshot.
The added optional pointer occupies eight bytes on a 64-bit host. Replacement
snapshots move to the heap rather than increasing the async rename frame.
Existing hosted async future-size caps remain unchanged and pass: mount 8192,
rename 3648, create 2304, set-label 3648 and extents 576 bytes.

The change adds no public API and preserves allocator-backed `no_std`, sync,
async and local I/O modes. It does not change allocator-free embedded recovery;
that implementation has its own focused correctness change.

## Verification

The hosted write and async suites pass **67 tests**, including seven added
regressions:

- The exact nonempty FAT32 rename cancelled at await budget five, checked by
  the raw oracle after sync and unmount.
- Every rename await across all five FAT geometries, covering same-directory
  and cross-directory nonempty files, replacement of nonempty targets,
  directory moves and replacement of empty directories. Tests retain source
  and target pins, exercise deferred dirty source metadata, verify contents
  and `..`, and independently check the resulting image.
- Stale short entries beyond End whose bytes exactly match the intended
  destination, plus unrelated trailing garbage, across FAT12/16/32. Every
  cancellation point must recover to exactly one source/destination and keep
  the garbage hidden.
- Every recovery await after cancellation late in a cross-directory move of
  a nonempty directory, followed by another sync, content checks and the oracle.
- Every initial replacement and recovery await for two empty files whose
  expected short metadata is identical. Retried rollback must retain the same
  outcome and pinned identities as uninterrupted recovery.
- Every failed rename write across all five geometries for nonempty file and
  directory moves, with and without replacement, followed by sync and the
  raw oracle.
- Repeated failures while removing a committed replacement's source. Pending
  recovery survives multiple failing sync calls and completes after the fault
  is cleared, preserving source contents and reclaiming the old target.

Existing failed-replace tests still require either the unchanged visible tree
or the completed rename after an injected failure; an old target cannot be
lost independently. Existing async resource guards also pass. The targeted
clippy run uses all targets and all features with warnings denied and the
repository's allowed legacy lints.

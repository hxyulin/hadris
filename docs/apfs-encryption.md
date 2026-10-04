# APFS encryption and the Asahi read-only driver

The driver supports password-unlocked, software-encrypted single-key APFS volumes
through the opt-in `encryption` feature (`apfs-encryption` in the umbrella crate).
Native macOS tools provide the independent producer and reader oracle. This is
one part of the eventual Asahi FUSE goal; it does not establish access to an
Apple-silicon Mac's internal FileVault Data volume.

## Two encryption paths

| Target | Decryption path | What a disposable image can prove |
|---|---|---|
| Password-encrypted APFS on a disk image or external disk | Software APFS key handling and data decryption | Password validation, key selection, decrypted metadata and file reads |
| Internal Apple-silicon APFS Data volume | Secure Enclave key handling and NVMe inline encryption | APFS reader behavior only; the image cannot qualify the hardware unlock path |

Apple describes internal FileVault key handling as occurring in the Secure
Enclave, without exposing encryption keys directly to the CPU. Asahi documents
the Secure Enclave passing volume keys directly to the NVMe controller for
encryption and decryption in flight. A software AES implementation with the
user's password is therefore insufficient evidence for internal-volume access.
The Linux SEP/NVMe unlock path needs separate qualification.

- [Apple: Volume encryption with FileVault](https://support.apple.com/guide/security/volume-encryption-with-filevault-sec4c6dc1b6e/web)
- [Asahi: Apple Silicon Platform Security](https://asahilinux.org/docs/platform/security/)
- [Apple File System Reference](https://developer.apple.com/support/apple-file-system/Apple-File-System-Reference.pdf)

## Reproducible native fixtures

Run on macOS, from the repository root:

```sh
cargo build -p hadris-cli
python3 scripts/apfs-encryption-fixtures.py \
  --output /tmp/hadris-apfs-encryption-fixtures \
  --hadris target/debug/hadris --encrypted-reads
```

The output directory must not exist. The script creates two disposable 64 MiB
APFS images without partition maps. It attaches only the images it created,
checks that each container's physical store belongs to the attachment, and
detaches in cleanup. It does not accept a device or existing container as input.

The encrypted fixture is created with `diskutil apfs addVolume` and a public
test password supplied over standard input. The temporary initial volume is
removed only from the new image; this also accommodates images whose container
allows only one volume. No DMG wrapper encryption is used. Both volumes contain
identical deterministic files, a sparse file, a hard link and a symlink.

The script checks that the encrypted volume is locked, rejects the wrong
password, accepts the correct password, and locks again after image detachment
and reattachment. A native read-only remount must reproduce file hashes, link
identity and symlink targets. `diskutil verifyVolume` runs while that read-only
mount is available, so the native checker can access its crypto I/O context.

`manifest.json` records the public password, volume UUIDs, native file hashes,
macOS version and passed checks. Native verifier and crypto-user output are
retained alongside the images. Binary fixtures stay outside the repository.
On failure, partial artifacts remain for diagnosis; rerun with a new directory.

The `--hadris --encrypted-reads` oracle requires plaintext and decrypted reads
to match every native hash, checks symlink following, explicit crypto-user
selection and generic extraction, and rejects absent or wrong credentials.
Use `--reuse` with an existing output directory to repeat only the driver checks;
these checks can also run on Linux because native production is already complete.
The manifest's public password is only for these deterministic test images.

## Unlock and read APIs

`ApfsFs::mount_with_password(device, options, password, crypto_user)` mounts a
sole software-encrypted volume. `mount_volume_with_password` adds a separate
`VolumeSelector`. Passwords are borrowed byte slices, and an optional crypto-user
UUID restricts the password-record search. Wrong credentials return
`ErrorKind::InvalidInput` with `Detail::Credentials`; mount failures return the
device. Generic `MountOptions` does not contain APFS credentials.

Native readers use `Container::unlock_volume`. Its key is tied to one volume,
replaced on an unlock attempt, redacted in `Debug`, and wiped on drop. Candidate
keys remain local until verification succeeds, including across async
cancellation. `read_volume_extents_at` and `read_volume_btree_node_with_flags`
require the volume explicitly; unscoped native reads reject encrypted inputs.
Mappings and extents must come from that volume's metadata.

Both modes share software crypto and parsing. Async APIs await device I/O;
password derivation itself is bounded CPU work performed within the mount call.
Keybags are capped at 1 MiB each and 256 records, and the entire password search
is capped at 1,000,000 PBKDF2 iterations. Unsupported individual wrapping formats
are skipped during automatic crypto-user selection, while malformed and failed
record-HMAC checks remain fatal. Explicitly selected unsupported users fail.

The implementation supports `APFS_FS_ONEKEY` software volumes. Hardware and
per-file-key encryption are rejected; encryption rolling is not qualified.
Password-based PBKDF2-SHA256, AES-KW and 512-byte AES-XTS are implemented with
RustCrypto primitives and mode dependencies. RFC3394, PBKDF2 and IEEE XTS known
answers accompany native image tests. DER record HMACs and object checksums are
validated; these do not authenticate the overall filesystem against an attacker.

The CLI accepts `--password-stdin` for `ls`, `stat`, `cat` and `extract`, plus
optional `--crypto-user <uuid>`. It reads one byte line and removes only LF/CRLF;
the password buffer is wiped on drop. Passwords are not accepted in arguments.

## Use cases for the unlock API

| Scenario | Required outcome |
|---|---|
| Correct disk-user password | Mount, enumerate metadata and reproduce native file hashes |
| Incorrect or absent password | Refuse access before exposing decrypted filesystem state; retain the device on mount failure |
| Multiple crypto users or keys | Select the intended identity without silently choosing an unrelated key |
| Sparse, linked and fragmented files | Decrypt allocated extents using their crypto identifiers and preserve holes and inode identity |
| Damaged keybag, oversized KDF parameters or unsupported protection | Return a bounded, typed failure without uncontrolled allocation or KDF work |
| Sync, async and `no_std` with allocation | Share parsing and crypto logic; keep host prompting and keychain access outside the library |
| Internal storage on Asahi | Accept an appropriate storage/unlock backend; do not assume a portable software volume key exists |

Decrypted metadata still passes the ordinary APFS structural and bounds checks.
Native tests cover sync/async mounts, sparse and offset reads, hard links,
symlinks, explicit identities, credential failures, key invalidation and async
unlock cancellation. Synthetic regressions cover malformed DER and keybags,
HMAC tampering, excessive KDF work and unsupported wrapping formats.

Volume selection stays independent of credentials. The storage backend remains responsible for hardware inline
decryption; APFS software encryption remains format-specific.

After software encryption, the FUSE qualification work still includes xattrs
and compression used by ordinary macOS files, Unicode name comparison, scalable
metadata traversal, and real Asahi hardware unlock tests. Start with a read-only
FUSE mount of an explicitly selected Data volume; presenting macOS's combined
System/Data namespace is a separate operation.

#!/usr/bin/env bash
# gen-seeds.sh — generate real seed images into fuzz/corpus/<target>/.
#
# Idempotent: seeds use fixed names and are overwritten in place; fuzzer-grown
# corpus entries (hash-named files) are never touched. Safe to re-run.
#
# Expected tools on the fuzz machine (Ubuntu 24): mkfs.vfat, mkntfs, cpio,
# python3. Missing tools skip their section with a notice.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

CORPUS="fuzz/corpus"
TARGETS="fat_read exfat_read ntfs_read iso_read udf_read cpio_read part_read fat_ops"
for t in $TARGETS; do
    mkdir -p "$CORPUS/$t"
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

have() { command -v "$1" >/dev/null 2>&1; }
note() { printf 'gen-seeds: %s\n' "$*"; }

if ! have python3; then
    note "python3 not found; crafted-image sections will be skipped"
fi

# --- fat_read -------------------------------------------------------------
if have mkfs.vfat; then
    mkfs.vfat -F 12 -C "$TMP/fat12-empty.img" 1440 >/dev/null
    mkfs.vfat -F 16 -C "$TMP/fat16-empty.img" 16384 >/dev/null
    mkfs.vfat -F 32 -C "$TMP/fat32-empty.img" 65536 >/dev/null
    cp "$TMP/fat12-empty.img" "$TMP/fat16-empty.img" "$TMP/fat32-empty.img" \
        "$CORPUS/fat_read/"
    note "fat_read: mkfs.vfat empty FAT12/16/32 images"
else
    note "fat_read: mkfs.vfat not found, skipping empty mkfs variants"
fi
if have python3; then
    python3 - "$TMP" <<'PYEOF'
import struct, sys
out = sys.argv[1] + "/fat12-crafted.img"
bps, total, spf, root_entries = 512, 1440, 5, 112
img = bytearray(bps * total)
img[0:3] = b"\xeb\x3c\x90"
img[3:11] = b"HADRIS  "
struct.pack_into("<H", img, 11, bps)
img[13] = 1
struct.pack_into("<H", img, 14, 1)
img[16] = 2
struct.pack_into("<H", img, 17, root_entries)
struct.pack_into("<H", img, 19, total)
img[21] = 0xF8
struct.pack_into("<H", img, 22, spf)
struct.pack_into("<H", img, 24, 9)
struct.pack_into("<H", img, 26, 2)
img[36] = 0x80
img[38] = 0x29
struct.pack_into("<I", img, 39, 0x1234ABCD)
img[43:54] = b"SEED       "
img[54:62] = b"FAT12   "
img[510:512] = b"\x55\xaa"
for fat in range(2):
    off = bps * (1 + fat * spf)
    img[off:off + 6] = b"\xf8\xff\xff\xff\xff\xff"  # clusters 2,3 -> EOF
root_off = bps * (1 + 2 * spf)
data_off = bps * (1 + 2 * spf + root_entries * 32 // bps)

def dir_entry(off, name, ext, attr, cluster, size):
    e = bytearray(32)
    e[0:8] = name.ljust(8)
    e[8:11] = ext.ljust(3)
    e[11] = attr
    struct.pack_into("<H", e, 26, cluster)
    struct.pack_into("<I", e, 28, size)
    img[off:off + 32] = e

content = b"hello from hadris seed\n"
dir_entry(root_off, b"HELLO", b"TXT", 0x20, 2, len(content))
dir_entry(root_off + 32, b"SUBDIR", b"", 0x10, 3, 0)
img[data_off:data_off + len(content)] = content
sub = data_off + bps
dir_entry(sub, b".", b"", 0x10, 3, 0)
dir_entry(sub + 32, b"..", b"", 0x10, 0, 0)
open(out, "wb").write(img)
PYEOF
    cp "$TMP/fat12-crafted.img" "$CORPUS/fat_read/"
    note "fat_read: hand-crafted FAT12 with root-dir entries"
fi

# --- exfat_read -----------------------------------------------------------
# No mkfs.exfat on the fuzz machine; reuse repo fixtures instead.
found_exfat=0
while IFS= read -r -d '' img; do
    cp "$img" "$CORPUS/exfat_read/$(basename "$img")"
    note "exfat_read: copied $img"
    found_exfat=1
done < <(find test-images crates -iname '*exfat*' \( -name '*.img' -o -name '*.bin' \) \
    -type f -print0 2>/dev/null)
if [ "$found_exfat" -eq 0 ]; then
    note "exfat_read: no exFAT images found in repo, skipping"
fi

# --- ntfs_read ------------------------------------------------------------
if have mkntfs; then
    truncate -s 8M "$TMP/ntfs-default.img"
    mkntfs -Q -F "$TMP/ntfs-default.img" >/dev/null
    truncate -s 8M "$TMP/ntfs-c4096.img"
    mkntfs -Q -F -c 4096 "$TMP/ntfs-c4096.img" >/dev/null
    truncate -s 8M "$TMP/ntfs-c512.img"
    mkntfs -Q -F -c 512 "$TMP/ntfs-c512.img" >/dev/null
    cp "$TMP"/ntfs-*.img "$CORPUS/ntfs_read/"
    note "ntfs_read: mkntfs images (default, 4K and 512B clusters)"
else
    note "ntfs_read: mkntfs not found, skipping"
fi

# --- cpio_read ------------------------------------------------------------
if have cpio; then
    mkdir -p "$TMP/tree/sub dir"
    printf 'hello from hadris seed\n' > "$TMP/tree/hello.txt"
    head -c 4096 /dev/urandom > "$TMP/tree/blob.bin" 2>/dev/null \
        || dd if=/dev/urandom of="$TMP/tree/blob.bin" bs=4096 count=1 2>/dev/null
    printf 'nested\n' > "$TMP/tree/sub dir/nested.txt"
    (cd "$TMP/tree" && find . | cpio -o -H newc --quiet > "$TMP/seed-newc.cpio")
    cp "$TMP/seed-newc.cpio" "$CORPUS/cpio_read/"
    note "cpio_read: newc archive from a temp tree"
    if echo x | cpio -o -H crc --quiet >/dev/null 2>&1; then
        (cd "$TMP/tree" && find . | cpio -o -H crc --quiet > "$TMP/seed-crc.cpio")
        cp "$TMP/seed-crc.cpio" "$CORPUS/cpio_read/"
        note "cpio_read: crc archive from a temp tree"
    else
        note "cpio_read: cpio lacks -H crc, skipping crc variant"
    fi
else
    note "cpio_read: cpio not found, skipping"
fi

# --- part_read ------------------------------------------------------------
if have python3; then
    python3 - "$TMP" <<'PYEOF'
import binascii, struct, sys, uuid
out = sys.argv[1]

def mbr(path, nparts):
    secs = 8192  # 4 MiB
    img = bytearray(512 * secs)
    types = [0x83, 0x07, 0x0B, 0x82]
    start = 2048
    for i in range(nparts):
        size = 1024
        e = bytes([0x80 if i == 0 else 0, 0, 2, 0, types[i % 4], 0, 2, 0])
        e += struct.pack("<II", start, size)
        img[446 + 16 * i:446 + 16 * i + 16] = e
        start += size + 256
    img[510:512] = b"\x55\xaa"
    open(path, "wb").write(img)

def gpt(path):
    secs = 4096  # 2 MiB
    img = bytearray(512 * secs)
    img[446:462] = bytes([0, 0, 2, 0, 0xEE, 0xFF, 0xFF, 0xFF]) \
        + struct.pack("<II", 1, secs - 1)
    img[510:512] = b"\x55\xaa"
    nent, esz = 128, 128
    parts = [
        ("c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "EFI System", 2048, 3071),
        ("0fc63daf-8483-4772-8e79-3d69d8477de4", "rootfs", 3072, 4000),
    ]
    entries = bytearray(nent * esz)
    for i, (tg, name, first, last) in enumerate(parts):
        off = i * esz
        entries[off:off + 16] = uuid.UUID(tg).bytes_le
        entries[off + 16:off + 32] = uuid.uuid5(uuid.NAMESPACE_DNS, name).bytes_le
        struct.pack_into("<QQ", entries, off + 32, first, last)
        nm = name.encode("utf-16-le")[:72]
        entries[off + 56:off + 56 + len(nm)] = nm
    ecrc = binascii.crc32(entries) & 0xFFFFFFFF
    img[2 * 512:2 * 512 + len(entries)] = entries
    backup_off = (secs - 1 - (len(entries) + 511) // 512) * 512
    img[backup_off:backup_off + len(entries)] = entries

    def header(cur, bak, ent_lba):
        h = bytearray(512)
        h[0:8] = b"EFI PART"
        struct.pack_into("<I", h, 8, 0x00010000)
        struct.pack_into("<I", h, 12, 92)
        struct.pack_into("<Q", h, 24, cur)
        struct.pack_into("<Q", h, 32, bak)
        struct.pack_into("<Q", h, 40, 34)
        struct.pack_into("<Q", h, 48, secs - 34)
        h[56:72] = uuid.UUID("deadc0de-1234-5678-9abc-def012345678").bytes_le
        struct.pack_into("<Q", h, 72, ent_lba)
        struct.pack_into("<I", h, 80, nent)
        struct.pack_into("<I", h, 84, esz)
        struct.pack_into("<I", h, 88, ecrc)
        struct.pack_into("<I", h, 16, binascii.crc32(h[:92]) & 0xFFFFFFFF)
        return h

    img[512:1024] = header(1, secs - 1, 2)
    img[(secs - 1) * 512:secs * 512] = header(secs - 1, 1, backup_off // 512)
    open(path, "wb").write(img)

mbr(out + "/mbr-1part.img", 1)
mbr(out + "/mbr-2part.img", 2)
mbr(out + "/mbr-4part.img", 4)
gpt(out + "/gpt-2part.img")
PYEOF
    cp "$TMP"/mbr-*.img "$TMP"/gpt-*.img "$CORPUS/part_read/"
    note "part_read: python-crafted MBR (1/2/4 entries) and GPT disks"
fi

# --- iso_read -------------------------------------------------------------
found_iso=0
while IFS= read -r -d '' img; do
    cp "$img" "$CORPUS/iso_read/$(basename "$img")"
    note "iso_read: copied $img"
    found_iso=1
done < <(find test-images crates/optical -name '*.iso' -type f -size -11M -print0 \
    2>/dev/null)
if [ "$found_iso" -eq 0 ] && have python3; then
    python3 - "$TMP" <<'PYEOF'
import struct, sys
out = sys.argv[1] + "/minimal.iso"
bps, nsec = 2048, 24
root_lba, file_lba = 20, 21
img = bytearray(bps * nsec)
content = b"hello from hadris iso seed\n"

def both16(v): return struct.pack("<H", v) + struct.pack(">H", v)
def both32(v): return struct.pack("<I", v) + struct.pack(">I", v)

def dir_record(extent, size, flags, name, date=(125, 8, 17, 0, 0, 0, 0)):
    di = name if isinstance(name, bytes) else name.encode()
    rec = both32(extent) + both32(size) + bytes(date) + bytes([flags, 0, 0])
    rec += both16(1) + bytes([len(di)]) + di
    if len(di) % 2 == 0:
        rec += b"\0"
    return bytes([len(rec) + 2, 0]) + rec

pvd = bytearray(bps)
pvd[0:7] = b"\x01CD001\x01"
pvd[8:40] = b"HADRIS".ljust(32)
pvd[40:72] = b"HADRIS_SEED".ljust(32)
pvd[80:88] = both32(nsec)
pvd[120:124] = both16(1)
pvd[124:128] = both16(1)
pvd[128:132] = both16(bps)
root_rec = dir_record(root_lba, bps, 2, b"\x00")
pt = bytes([1, 0]) + struct.pack("<I", root_lba) + struct.pack("<H", 1) + b"\x00\x00"
pvd[132:140] = struct.pack("<I", len(pt)) + struct.pack(">I", len(pt))
struct.pack_into("<I", pvd, 140, 18)
struct.pack_into("<I", pvd, 148, 19)
pvd[156:156 + len(root_rec)] = root_rec
pvd[190:318] = b"HADRIS SEED VOLUME".ljust(128)
img[16 * bps:17 * bps] = pvd
img[17 * bps:17 * bps + 7] = b"\xffCD001\x01"
img[18 * bps:18 * bps + len(pt)] = pt
pt_m = bytes([1, 0]) + struct.pack(">I", root_lba) + struct.pack(">H", 1) + b"\x00\x00"
img[19 * bps:19 * bps + len(pt_m)] = pt_m
root = dir_record(root_lba, bps, 2, b"\x00") \
    + dir_record(root_lba, bps, 2, b"\x01") \
    + dir_record(file_lba, len(content), 0, "HELLO.TXT;1")
img[root_lba * bps:root_lba * bps + len(root)] = root
img[file_lba * bps:file_lba * bps + len(content)] = content
open(out, "wb").write(img)
PYEOF
    cp "$TMP/minimal.iso" "$CORPUS/iso_read/"
    note "iso_read: python-crafted minimal ISO9660 (no .iso fixtures found)"
elif [ "$found_iso" -eq 0 ]; then
    note "iso_read: no .iso fixtures and no python3, skipping"
fi

# --- udf_read -------------------------------------------------------------
found_udf=0
while IFS= read -r -d '' img; do
    cp "$img" "$CORPUS/udf_read/$(basename "$img")"
    note "udf_read: copied $img"
    found_udf=1
done < <(find test-images crates/optical -name '*.udf' -type f -size -11M -print0 \
    2>/dev/null)
if [ "$found_udf" -eq 0 ]; then
    note "udf_read: no .udf images found in repo, skipping"
fi

# --- summary --------------------------------------------------------------
echo
note "corpus summary:"
for t in $TARGETS; do
    n=$(find "$CORPUS/$t" -type f | wc -l | tr -d ' ')
    printf '  %-12s %s files\n' "$t" "$n"
done

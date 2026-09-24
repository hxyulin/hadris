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
TARGETS="fat_read exfat_read ntfs_read iso_read udf_read cpio_read part_read fat_ops exfat_ops"
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
# Every seed stays within the 64 KiB max_len of part_read, so the fuzzer sees
# the whole disk, backup GPT included.
if have python3; then
    python3 - "$TMP" <<'PYEOF'
import binascii, struct, sys, uuid
out = sys.argv[1]

def entry(img, at, kind, start, count, boot=0):
    img[at:at + 16] = bytes([boot, 0, 0, 0, kind, 0, 0, 0]) + struct.pack("<II", start, count)

def signed(img, block, bs=512):
    img[block * bs + 510:block * bs + 512] = b"\x55\xaa"

def mbr(path):
    img = bytearray(512 * 64)
    entry(img, 446, 0x0C, 2, 30, boot=0x80)
    entry(img, 462, 0x83, 40, 24)
    signed(img, 0)
    open(path, "wb").write(img)

def ebr(path):
    # Extended partition 16..96; EBRs at 16, 40 and 60 chain three
    # logical partitions.
    img = bytearray(512 * 96)
    entry(img, 446, 0x0C, 2, 10)
    entry(img, 462, 0x0F, 16, 80)
    signed(img, 0)
    chain = [(16, 18, 10, 40), (40, 42, 10, 60), (60, 62, 20, None)]
    for ebr_lba, start, count, nxt in chain:
        at = ebr_lba * 512
        entry(img, at + 446, 0x83, start - ebr_lba, count)
        if nxt is not None:
            nxt_end = [c for c in chain if c[0] == nxt][0]
            entry(img, at + 462, 0x05, nxt - 16, nxt_end[1] + nxt_end[2] - nxt)
        signed(img, ebr_lba)
    open(path, "wb").write(img)

def gpt(path, bs=512, blocks=48, hybrid=False, damage=None):
    img = bytearray(bs * blocks)
    nent, esz = 4, 128
    array_blocks = (nent * esz + bs - 1) // bs
    first, last = 2 + array_blocks, blocks - 2 - array_blocks
    esp_last = first + (last - first) // 3
    parts = [
        ("c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "EFI System", first, esp_last, 1),
        ("0fc63daf-8483-4772-8e79-3d69d8477de4", "root é\U0001f600", esp_last + 1, last, 4),
    ]
    if hybrid:
        entry(img, 446, 0xEE, 1, first - 1)
        entry(img, 462, 0xEF, first, esp_last - first + 1, boot=0x80)
    else:
        entry(img, 446, 0xEE, 1, blocks - 1)
    signed(img, 0, bs)
    entries = bytearray(nent * esz)
    for i, (tg, name, lo, hi, attrs) in enumerate(parts):
        off = i * esz
        entries[off:off + 16] = uuid.UUID(tg).bytes_le
        entries[off + 16:off + 32] = uuid.uuid5(uuid.NAMESPACE_DNS, name).bytes_le
        struct.pack_into("<QQQ", entries, off + 32, lo, hi, attrs)
        nm = name.encode("utf-16-le")[:72]
        entries[off + 56:off + 56 + len(nm)] = nm
    ecrc = binascii.crc32(entries) & 0xFFFFFFFF
    backup_array = blocks - 1 - array_blocks
    img[2 * bs:2 * bs + len(entries)] = entries
    img[backup_array * bs:backup_array * bs + len(entries)] = entries

    def header(cur, alt, ent_lba):
        h = bytearray(92)
        h[0:8] = b"EFI PART"
        struct.pack_into("<IIIIQQQQ", h, 8, 0x00010000, 92, 0, 0, cur, alt, first, last)
        h[56:72] = uuid.UUID("deadc0de-1234-5678-9abc-def012345678").bytes_le
        struct.pack_into("<QIII", h, 72, ent_lba, nent, esz, ecrc)
        struct.pack_into("<I", h, 16, binascii.crc32(h) & 0xFFFFFFFF)
        return h

    img[bs:bs + 92] = header(1, blocks - 1, 2)
    img[(blocks - 1) * bs:(blocks - 1) * bs + 92] = header(blocks - 1, 1, backup_array)
    if damage == "primary":
        img[bs] ^= 0xFF
    elif damage == "backup":
        img[(blocks - 1) * bs + 24] ^= 0xFF
    open(path, "wb").write(img)

mbr(out + "/part-mbr.img")
ebr(out + "/part-ebr.img")
gpt(out + "/part-gpt.img")
gpt(out + "/part-gpt-4k.img", bs=4096, blocks=8)
gpt(out + "/part-hybrid.img", hybrid=True)
gpt(out + "/part-gpt-primary-damaged.img", damage="primary")
gpt(out + "/part-gpt-backup-damaged.img", damage="backup")
PYEOF
    cp "$TMP"/part-*.img "$CORPUS/part_read/"
    note "part_read: python-crafted MBR, EBR chain, GPT (512 and 4096), hybrid and damaged GPT disks"
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

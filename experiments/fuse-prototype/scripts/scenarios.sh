#!/usr/bin/env bash
# Mount scenarios for the FUSE prototype. Runs as root on Linux with fuse3
# and dosfstools installed; scripts/docker.sh sets that up.
#
#   scenarios.sh <path to hadris-fuse-prototype>
#
# Each scenario prints PASS, FAIL, or GAP (a known difference from POSIX
# that follows from the hadris-fs contract, recorded rather than failed).
set -uo pipefail

BIN="$1"
WORK="$(mktemp -d)"
IMG="$WORK/fat.img"
M="$WORK/mnt"
SRC="$WORK/src"
LOG="$WORK/logs"
mkdir -p "$M" "$SRC" "$LOG"
PID=""
FAILED=0

cat >"$WORK/renameat2.c" <<'C'
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>
int main(int argc, char **argv) {
    unsigned flags = strtoul(argv[3], 0, 0);
    if (syscall(SYS_renameat2, AT_FDCWD, argv[1], AT_FDCWD, argv[2], flags)) {
        printf("%s\n", strerror(errno));
        return 1;
    }
    printf("ok\n");
    return 0;
}
C
gcc -o "$WORK/renameat2" "$WORK/renameat2.c"

mount_fs() {
    "$BIN" mount "$IMG" "$M" --threads 4 2>"$LOG/daemon.log" &
    PID=$!
    for _ in $(seq 1 50); do
        mountpoint -q "$M" && return 0
        sleep 0.1
    done
    echo "mount did not come up"
    cat "$LOG/daemon.log"
    exit 1
}

unmount_fs() {
    umount "$M" && wait "$PID"
}

scenario() {
    local name="$1"
    shift
    local out="$LOG/$name.log"
    (set -e; "$@") >"$out" 2>&1
    local code=$?
    if [ "$code" = 0 ]; then
        printf 'PASS  %s\n' "$name"
    else
        if [ "$code" = 42 ]; then
            printf 'GAP   %s: %s\n' "$name" "$(tail -1 "$out")"
        else
            printf 'FAIL  %s\n' "$name"
            sed 's/^/      /' "$out" | tail -8
            FAILED=1
        fi
    fi
}

gap() {
    echo "$*"
    exit 42
}

mkdir_tree() {
    mkdir -p "$M/tree/a/b/c/d/e/f"
    mkdir -p "$M/tree/a/x/y"
    test -d "$M/tree/a/b/c/d/e/f"
    [ "$(find "$M/tree" -type d | wc -l)" = 9 ]
}

copy_in() {
    head -c 0 /dev/urandom >"$SRC/empty"
    head -c 1 /dev/urandom >"$SRC/one"
    head -c 511 /dev/urandom >"$SRC/odd"
    head -c 4096 /dev/urandom >"$SRC/block"
    head -c 100000 /dev/urandom >"$SRC/medium"
    head -c 3000000 /dev/urandom >"$SRC/large"
    mkdir -p "$SRC/sub/deeper"
    head -c 12345 /dev/urandom >"$SRC/sub/deeper/nested.bin"
    cp -r "$SRC" "$M/src"
    for f in empty one odd block medium large sub/deeper/nested.bin; do
        cmp "$SRC/$f" "$M/src/$f"
        cat "$M/src/$f" | cmp - "$SRC/$f"
    done
    (cd "$SRC" && sha256sum empty odd block medium large sub/deeper/nested.bin) >"$WORK/src.sha"
}

copy_preserve() {
    cp -p "$SRC/medium" "$M/src/medium.p"
    cmp "$SRC/medium" "$M/src/medium.p"
    [ "$(stat -c %Y "$SRC/medium")" -le "$(( $(stat -c %Y "$M/src/medium.p") + 1 ))" ]
}

large_dir() {
    mkdir "$M/big"
    for i in $(seq -w 1 3000); do
        : >"$M/big/file-with-a-long-name-$i.txt"
    done
    local start end lines
    echo 3 >/proc/sys/vm/drop_caches || true
    start=$(date +%s%N)
    ls -f "$M/big" >/dev/null
    end=$(date +%s%N)
    echo "ls -f of 3000 entries: $(( (end - start) / 1000000 )) ms"
    start=$(date +%s%N)
    lines=$(ls -la "$M/big" | wc -l)
    end=$(date +%s%N)
    echo "ls -la of 3000 entries: $(( (end - start) / 1000000 )) ms"
    [ "$lines" = 3003 ]
    [ "$(ls -f "$M/big" | sort | uniq -d | wc -l)" = 0 ]
}

mv_rename() {
    mv "$M/src/one" "$M/src/uno"
    [ ! -e "$M/src/one" ]
    cmp "$SRC/one" "$M/src/uno"
    mv "$M/src/uno" "$M/tree/a/b/uno"
    cmp "$SRC/one" "$M/tree/a/b/uno"
    mv "$M/tree/a/x" "$M/tree/x-moved"
    test -d "$M/tree/x-moved/y"
}

mv_overwrite() {
    echo AAA >"$M/ow-a"
    echo BBB >"$M/ow-b"
    cat "$M/ow-b" >/dev/null
    mv "$M/ow-a" "$M/ow-b"
    [ "$(cat "$M/ow-b")" = AAA ]
    [ ! -e "$M/ow-a" ]
    mkdir "$M/ow-d1" "$M/ow-d2"
    ls "$M/ow-d2" >/dev/null
    mv -T "$M/ow-d1" "$M/ow-d2"
    [ ! -e "$M/ow-d1" ]
}

rename_flags() {
    echo 1 >"$M/rf-a"
    echo 2 >"$M/rf-b"
    [ "$("$WORK/renameat2" "$M/rf-a" "$M/rf-b" 1)" = "File exists" ]
    [ "$(cat "$M/rf-b")" = 2 ]
    [ "$("$WORK/renameat2" "$M/rf-a" "$M/rf-c" 1)" = ok ]
    [ "$("$WORK/renameat2" "$M/rf-c" "$M/rf-b" 2)" = "Invalid argument" ]
}

case_insensitive() {
    echo x >"$M/CaseName.TXT"
    [ "$(cat "$M/casename.txt")" = x ]
    [ "$(stat -c %i "$M/CaseName.TXT")" = "$(stat -c %i "$M/casename.txt")" ]
    mv "$M/CaseName.TXT" "$M/tmp-name"
    mv "$M/tmp-name" "$M/casename.txt"
    ls "$M" | grep -qx casename.txt
    echo y >"$M/Other.TXT"
    [ "$("$WORK/renameat2" "$M/Other.TXT" "$M/other.txt" 0)" = ok ]
    ls "$M" | grep -qx other.txt || gap "a case-only rename never reaches the driver: the VFS sees one inode under both names"
}

rm_recursive() {
    cp -r "$M/src" "$M/doomed"
    ls -laR "$M/doomed" >/dev/null
    rm -r "$M/doomed"
    [ ! -e "$M/doomed" ]
    rm -r "$M/tree/a"
    [ "$(ls "$M/tree")" = x-moved ]
}

truncate_file() {
    cp "$SRC/medium" "$M/trunc"
    truncate -s 10 "$M/trunc"
    [ "$(stat -c %s "$M/trunc")" = 10 ]
    cmp -n 10 "$SRC/medium" "$M/trunc"
    truncate -s 1M "$M/trunc"
    [ "$(stat -c %s "$M/trunc")" = 1048576 ]
    [ "$(tail -c +11 "$M/trunc" | tr -d '\0' | wc -c)" = 0 ]
    truncate -s 0 "$M/trunc"
    [ "$(stat -c %s "$M/trunc")" = 0 ]
}

touch_times() {
    touch "$M/t"
    touch -m -d @1577934246 "$M/t"
    [ "$(stat -c %Y "$M/t")" = 1577934246 ]
    touch -m -d @1577934247 "$M/t"
    echo "odd second stored as $(stat -c %Y "$M/t") (2 s resolution)"
    touch -a -d @1577934246 "$M/t"
    echo "atime stored as $(stat -c %X "$M/t") (FAT keeps the date only)"
}

concurrent_readers() {
    local pids=()
    for i in $(seq 1 8); do
        cmp "$SRC/large" "$M/src/large" &
        pids+=($!)
        cmp "$SRC/medium" "$M/src/medium" &
        pids+=($!)
    done
    for p in "${pids[@]}"; do wait "$p"; done
}

find_tree() {
    [ "$(find "$M/src" -type f | wc -l)" = "$(find "$SRC" -type f | wc -l)" ]
    [ "$(find "$M/big" -name 'file-with-a-long-name-29*' | wc -l)" = 100 ]
    find "$M" -type f -size +1M | grep -q large
}

statfs_df() {
    df "$M"
    [ "$(stat -f -c %b "$M")" -gt 0 ]
}

readdir_while_creating() {
    mkdir "$M/race"
    for i in $(seq 1 200); do : >"$M/race/old-$i"; done
    (for i in $(seq 1 500); do : >"$M/race/new-$i"; done) &
    local creator=$!
    local listings=0
    while kill -0 "$creator" 2>/dev/null; do
        ls -f "$M/race" >"$WORK/race.ls"
        [ "$(sort "$WORK/race.ls" | uniq -d | wc -l)" = 0 ]
        [ "$(grep -c '^old-' "$WORK/race.ls")" = 200 ]
        listings=$((listings + 1))
    done
    wait "$creator"
    echo "$listings listings taken while creating"
    [ "$(ls "$M/race" | wc -l)" = 700 ]
}

inode_numbers() {
    echo a >"$M/ino-a"
    mv "$M/ino-a" "$M/ino-b"
    exec 7<"$M/ino-b"
    echo c >"$M/ino-c"
    sync
    if echo 3 >/proc/sys/vm/drop_caches; then echo "dropped caches"; fi
    local listed stat
    listed=$(python3 -c 'import os,sys; print([e.inode() for e in os.scandir(sys.argv[1]) if e.name == "ino-c"][0])' "$M")
    stat=$(stat -c %i "$M/ino-c")
    exec 7<&-
    echo "ino-c: readdir $listed, stat $stat"
    [ "$listed" = "$stat" ] || gap "readdir and stat disagree on the inode of ino-c ($listed vs $stat)"
}

unlink_open_file() {
    echo data >"$M/held"
    exec 5<"$M/held"
    local rc=0
    rm "$M/held" 2>"$WORK/rm.err" || rc=$?
    exec 5<&-
    if [ "$rc" != 0 ]; then
        rm "$M/held"
        gap "rm of an open file: $(cat "$WORK/rm.err")"
    fi
}

rename_over_open_file() {
    echo new >"$M/ro-new"
    echo old >"$M/ro-old"
    exec 5<"$M/ro-old"
    local rc=0
    mv "$M/ro-new" "$M/ro-old" 2>"$WORK/mv.err" || rc=$?
    exec 5<&-
    [ "$rc" = 0 ] || gap "mv over an open file: $(cat "$WORK/mv.err")"
}

symlink_and_link() {
    local out
    out=$(ln -s target "$M/sym" 2>&1) && return 1
    echo "symlink: $out"
    out=$(ln "$M/ow-b" "$M/hard" 2>&1) && return 1
    echo "link: $out"
    gap "no symlinks or hard links on FAT (EPERM)"
}

unmount_with_open_file() {
    exec 6>"$M/open-at-umount"
    echo first >&6
    if umount "$M" 2>"$WORK/umount.err"; then
        echo "plain umount succeeded with a file open"
        return 1
    fi
    echo "plain umount: $(cat "$WORK/umount.err")"
    umount -l "$M"
    echo second >&6
    exec 6>&-
    for _ in $(seq 1 100); do
        grep -q '^State:.*Z' "/proc/$PID/status" 2>/dev/null || [ ! -d "/proc/$PID" ] && break
        sleep 0.1
    done
    if [ -d "/proc/$PID" ] && ! grep -q '^State:.*Z' "/proc/$PID/status"; then
        echo "daemon still running after the last file closed"
        return 1
    fi
    echo "daemon exited after the last file closed"
}

fsck_image() {
    fsck.fat -n "$IMG"
}

remount_and_verify() {
    mount_fs
    [ "$(cat "$M/open-at-umount")" = "$(printf 'first\nsecond')" ]
    (cd "$M/src" && sha256sum -c --quiet "$WORK/src.sha")
    [ "$(ls "$M/big" | wc -l)" = 3000 ]
    [ "$(ls "$M/race" | wc -l)" = 700 ]
    unmount_fs
}

"$BIN" format "$IMG" $((256 << 20)) fat32
mount_fs
echo "mounted $IMG (FAT32, 256 MiB) at $M"

scenario mkdir_tree mkdir_tree
scenario copy_in_cat_cmp copy_in
scenario copy_preserve copy_preserve
scenario large_dir_ls_la large_dir
scenario mv_rename mv_rename
scenario mv_overwrite mv_overwrite
scenario renameat2_flags rename_flags
scenario case_insensitive case_insensitive
scenario rm_recursive rm_recursive
scenario truncate truncate_file
scenario touch_times touch_times
scenario concurrent_readers concurrent_readers
scenario find find_tree
scenario statfs_df statfs_df
scenario readdir_while_creating readdir_while_creating
scenario inode_numbers inode_numbers
scenario unlink_open_file unlink_open_file
scenario rename_over_open_file rename_over_open_file
scenario symlink_and_link symlink_and_link
scenario unmount_with_open_file unmount_with_open_file
wait "$PID"
scenario fsck_after_unmount fsck_image
scenario remount_and_verify remount_and_verify
scenario fsck_final fsck_image

echo
for f in "$LOG"/large_dir_ls_la.log "$LOG"/touch_times.log "$LOG"/readdir_while_creating.log \
    "$LOG"/unmount_with_open_file.log "$LOG"/inode_numbers.log; do
    [ -s "$f" ] && sed "s|^|$(basename "$f" .log): |" "$f"
done
if [ -s "$LOG/daemon.log" ]; then
    echo "daemon stderr:"
    sort "$LOG/daemon.log" | uniq -c | sort -rn | head -20
fi
exit "$FAILED"

#!/usr/bin/env bash
# fuzz-fleet.sh — (re)launch the tmux fuzzing fleet from the repo root.
#
# Usage: fuzz-fleet.sh [session-name]   (default: fuzz)
#
# One tmux window per target, each running 4 forking libFuzzer jobs. Any
# existing session with the same name is killed first.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

SESSION="${1:-fuzz}"
TARGETS="fat_read exfat_read ntfs_read iso_read udf_read cpio_read part_read fat_ops"

max_len_for() {
    case "$1" in
        part_read | ntfs_read | cpio_read) echo 65536 ;;
        fat_read | exfat_read) echo 131072 ;;
        iso_read | udf_read) echo 1048576 ;;
        fat_ops) echo 4096 ;;
        *) echo 65536 ;;
    esac
}

if ! command -v tmux >/dev/null 2>&1; then
    echo "fuzz-fleet: tmux not found" >&2
    exit 1
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

first=1
for t in $TARGETS; do
    n="$(max_len_for "$t")"
    cmd="cd '$REPO_ROOT/fuzz' && cargo +nightly fuzz run $t -- -fork=4 -ignore_crashes=1 -rss_limit_mb=2048 -max_len=$n -len_control=0 -use_value_profile=1; echo; echo 'fuzz-fleet: $t exited'; exec bash"
    if [ "$first" -eq 1 ]; then
        tmux new-session -d -s "$SESSION" -n "$t" "$cmd"
        first=0
    else
        tmux new-window -t "$SESSION" -n "$t" "$cmd"
    fi
done

echo "fuzz-fleet: session '$SESSION' started with $(echo $TARGETS | wc -w | tr -d ' ') windows"
echo "  attach:    tmux attach -t $SESSION"
echo "  kill:      tmux kill-session -t $SESSION"
echo "  artifacts: ls fuzz/artifacts/<target>/"

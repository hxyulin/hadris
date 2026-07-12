#!/usr/bin/env bash
set -euo pipefail

mode="${1:-check}"
case "$mode" in
  check|update) ;;
  *) echo "usage: $0 [check|update]" >&2; exit 2 ;;
esac

if ! cargo public-api --version >/dev/null 2>&1; then
  echo "cargo-public-api is required (CI uses version 0.52.0)" >&2
  exit 1
fi

crates=(
  hadris
  hadris-archive
  hadris-block
  hadris-cd
  hadris-common
  hadris-cpio
  hadris-fat
  hadris-fixed
  hadris-io
  hadris-iso
  hadris-macros
  hadris-optical
  hadris-part
  hadris-path
  hadris-storage
  hadris-udf
)

snapshot_dir="api-snapshots"
mkdir -p "$snapshot_dir"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

for crate in "${crates[@]}"; do
  generated="$tmp_dir/$crate.txt"
  cargo public-api -p "$crate" --all-features -sss --color never >"$generated"
  if [[ "$mode" == update ]]; then
    cp "$generated" "$snapshot_dir/$crate.txt"
  elif ! diff -u "$snapshot_dir/$crate.txt" "$generated"; then
    echo "public API snapshot changed for $crate" >&2
    echo "review it, then run scripts/check-public-api.sh update" >&2
    exit 1
  fi
done

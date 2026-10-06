#!/usr/bin/env bash
# Builds the no_std tiers for bare-metal targets (NF-TARGET-01). Every
# target builds the tiers without an allocator; targets with pointer-sized
# atomics also build the `alloc` tiers, which need `Arc`.
#
# usage: scripts/check-targets.sh [target...]
set -euo pipefail

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
  targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imc-unknown-none-elf)
fi

no_alloc=(
  "hadris-common:"
  "hadris-io:sync,async,embedded-io"
  "hadris-storage:sync,async"
  "hadris-fs:sync,async"
  "hadris-fat-raw:sync,async"
  "hadris-fat:sync"
  "hadris-fat:async"
  "hadris-fat:sync,async,write"
  "hadris-part:sync,async"
  "hadris-iso:sync,async"
  "hadris-udf:sync,async"
  "hadris-cpio:sync,async"
  "hadris-ntfs:sync,async"
  "hadris:sync,async,detect"
)
with_alloc=(
  "hadris-fs:alloc,sync,async"
  "hadris-fat:alloc,sync,async,write"
  "hadris-part:alloc,sync,async"
  "hadris-iso:alloc,sync,async"
  "hadris-udf:alloc,sync,async"
  "hadris-cpio:alloc,sync,async"
  "hadris-ntfs:alloc,sync,async"
  "hadris:alloc,sync,async,write,detect,part,cpio"
)

failed=0
for target in "${targets[@]}"; do
  tiers=("${no_alloc[@]}")
  if rustc --print cfg --target "$target" | grep -q 'target_has_atomic="ptr"'; then
    tiers+=("${with_alloc[@]}")
  fi
  for tier in "${tiers[@]}"; do
    crate="${tier%%:*}"
    features="${tier#*:}"
    if ! cargo build -q -p "$crate" --no-default-features --features "$features" --target "$target"; then
      echo "FAILED: $target $crate [$features]" >&2
      failed=1
    fi
  done
  echo "$target: ${#tiers[@]} tiers built"
done
exit "$failed"

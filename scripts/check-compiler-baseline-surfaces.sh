#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd -- "$(dirname -- "$BASH_SOURCE")/.." && pwd)"
status=0

matches="$(
  git -C "$root_directory" grep -n -E \
    'beta-[0-9]{4}-[0-9]{2}-[0-9]{2}|1\.98\.0-beta\.[0-9]+' \
    -- \
    ':(top,glob).github/workflows/**' \
    ':(top,glob)scripts/**' \
    ':(top,glob,exclude)scripts/**/test-*' \
    ':(top,glob,exclude)scripts/**/*.test.ts' \
    ':(top,glob,exclude)scripts/check-compiler-baseline-surfaces.sh' 2>/dev/null || true
)"
if [[ -n "$matches" ]]; then
  printf 'compiler baseline: hidden product literal(s) found:\n%s\n' "$matches" >&2
  status=1
fi

while IFS= read -r workflow; do
  if ! git -C "$root_directory" grep -q 'scripts/rust-baseline\.sh channel' -- "$workflow"; then
    printf 'compiler baseline: workflow does not derive its primary toolchain: %s\n' "$workflow" >&2
    status=1
  fi
done < <(git -C "$root_directory" ls-files '.github/workflows/*.yml')

if (( status == 0 )); then
  printf 'compiler baseline: hosted and packaging surfaces derive the canonical selector\n'
fi
exit "$status"

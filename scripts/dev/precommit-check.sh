#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
export CARGO_BUILD_JOBS=10
cd "$root"

log="$(mktemp "${TMPDIR:-/tmp}/rusttable-precommit.XXXXXX")"
cleanup() {
  rm -f "$log"
}
trap cleanup EXIT

print_failure_excerpt() {
  local line_count
  line_count="$(wc -l <"$log")"
  if (( line_count <= 80 )); then
    cat "$log"
    return
  fi
  sed -n '1,12p' "$log"
  printf '%s\n' '... output excerpt truncated ...'
  tail -n 67 "$log"
}

set +e
cargo xtask check --parallel >"$log" 2>&1
status=$?
set -e
if (( status != 0 )); then
  printf 'FAIL pre-commit check (exit=%s)\n' "$status" >&2
  print_failure_excerpt >&2
  exit "$status"
fi

printf 'PASS pre-commit check (CARGO_BUILD_JOBS=%s)\n' "$CARGO_BUILD_JOBS"

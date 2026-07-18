#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

cargo test --workspace --all-targets >"$temporary_directory/test.log" 2>&1 &
test_pid=$!
cargo clippy --workspace --all-targets --all-features -- -D warnings >"$temporary_directory/clippy.log" 2>&1 &
clippy_pid=$!

status=0
if ! wait "$test_pid"; then
  status=1
  cat "$temporary_directory/test.log" >&2
fi
if ! wait "$clippy_pid"; then
  status=1
  cat "$temporary_directory/clippy.log" >&2
fi

exit "$status"


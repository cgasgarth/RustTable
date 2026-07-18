#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
source "$(dirname "$0")/with-validation-budget.sh"

cargo test --workspace --all-targets --locked >"$temporary_directory/test.log" 2>&1 &
test_pid=$!
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings >"$temporary_directory/clippy.log" 2>&1 &
clippy_pid=$!
bun run test:computer-use >"$temporary_directory/computer-use.log" 2>&1 &
computer_use_pid=$!

run_checks() {
  local status=0
  if ! wait "$test_pid"; then
    status=1
    cat "$temporary_directory/test.log" >&2
  fi
  if ! wait "$clippy_pid"; then
    status=1
    cat "$temporary_directory/clippy.log" >&2
  fi
  if ! wait "$computer_use_pid"; then
    status=1
    cat "$temporary_directory/computer-use.log" >&2
  fi
  return "$status"
}

run_with_budget 60 run_checks

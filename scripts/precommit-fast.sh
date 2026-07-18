#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
source "$(dirname "$0")/with-validation-budget.sh"

cleanup() {
  local status="$?"
  if [[ -n "${budget_process_id:-}" ]]; then
    terminate_process_tree "$budget_process_id"
    wait "$budget_process_id" 2>/dev/null || true
  fi
  rm -rf "$temporary_directory"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run_checks() {
  local status=0
  cargo fmt --all -- --check >"$temporary_directory/fmt.log" 2>&1 &
  format_pid=$!
  cargo metadata --locked --no-deps --format-version 1 >"$temporary_directory/lock.log" 2>&1 &
  lock_pid=$!
  bash scripts/check-source-policy.sh >"$temporary_directory/policy.log" 2>&1 &
  policy_pid=$!
  bun run test:computer-use >"$temporary_directory/computer-use.log" 2>&1 &
  computer_use_pid=$!
  bash scripts/test-dependency-security.sh >"$temporary_directory/security.log" 2>&1 &
  security_pid=$!
  if ! wait "$format_pid"; then
    status=1
    cat "$temporary_directory/fmt.log" >&2
  fi
  if ! wait "$lock_pid"; then
    status=1
    cat "$temporary_directory/lock.log" >&2
  fi
  if ! wait "$policy_pid"; then
    status=1
    cat "$temporary_directory/policy.log" >&2
  fi
  if ! wait "$computer_use_pid"; then
    status=1
    cat "$temporary_directory/computer-use.log" >&2
  fi
  if ! wait "$security_pid"; then
    status=1
    cat "$temporary_directory/security.log" >&2
  fi
  return "$status"
}

run_with_budget 30 pre-commit run_checks

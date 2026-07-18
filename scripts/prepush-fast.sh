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
  cargo test --workspace --all-targets --all-features --locked >"$temporary_directory/test.log" 2>&1 &
  test_pid=$!
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings >"$temporary_directory/clippy.log" 2>&1 &
  clippy_pid=$!
  bun run test:cache-workflow-policy >"$temporary_directory/cache-workflow-policy.log" 2>&1 &
  cache_workflow_policy_pid=$!
  bun run check:cache-workflow-policy >"$temporary_directory/cache-workflow-policy-check.log" 2>&1 &
  cache_workflow_policy_check_pid=$!
  bun run check:workspace-rust-version >"$temporary_directory/workspace-rust-version.log" 2>&1 &
  workspace_rust_version_pid=$!
  bun run check:workspace-layout >"$temporary_directory/workspace-layout.log" 2>&1 &
  workspace_layout_pid=$!
  bun run test:computer-use >"$temporary_directory/computer-use.log" 2>&1 &
  computer_use_pid=$!
  if ! wait "$test_pid"; then
    status=1
    cat "$temporary_directory/test.log" >&2
  fi
  if ! wait "$clippy_pid"; then
    status=1
    cat "$temporary_directory/clippy.log" >&2
  fi
  if ! wait "$cache_workflow_policy_pid"; then
    status=1
    cat "$temporary_directory/cache-workflow-policy.log" >&2
  fi
  if ! wait "$cache_workflow_policy_check_pid"; then
    status=1
    cat "$temporary_directory/cache-workflow-policy-check.log" >&2
  fi
  if ! wait "$workspace_rust_version_pid"; then
    status=1
    cat "$temporary_directory/workspace-rust-version.log" >&2
  fi
  if ! wait "$workspace_layout_pid"; then
    status=1
    cat "$temporary_directory/workspace-layout.log" >&2
  fi
  if ! wait "$computer_use_pid"; then
    status=1
    cat "$temporary_directory/computer-use.log" >&2
  fi
  return "$status"
}

run_with_budget 60 pre-push run_checks

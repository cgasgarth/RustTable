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

summarize_failure() {
  local label="$1"
  local log_file="$2"
  local line_count

  printf 'pre-commit check failed: %s\n' "$label" >&2
  if [[ ! -s "$log_file" ]]; then
    printf '  (no output)\n' >&2
    return
  fi
  line_count="$(wc -l <"$log_file")"
  if (( line_count > 40 )); then
    printf '  ... showing the last 40 of %s lines ...\n' "$line_count" >&2
  fi
  tail -n 40 "$log_file" >&2
}

run_checks() {
  local status=0
  local index
  local label
  local log_file
  local -a check_labels=()
  local -a check_pids=()
  local -a check_logs=()

  start_check() {
    label="$1"
    shift
    check_labels+=("$label")
    log_file="$temporary_directory/$label.log"
    check_logs+=("$log_file")
    "$@" >"$log_file" 2>&1 &
    check_pids+=("$!")
  }

  start_check diff git diff --check
  start_check fmt cargo fmt --all -- --check
  start_check metadata cargo metadata --locked --no-deps --format-version 1
  start_check rust-check cargo check --workspace --all-targets --all-features --locked
  start_check rust-clippy cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  start_check rust-test cargo test --workspace --lib --all-features --locked
  start_check source bash scripts/check-source-policy.sh
  start_check computer-use bun run test:computer-use
  start_check macos-artifact-identity bun run test:macos-artifact-identity
  start_check doctor bash scripts/dev/test-doctor.sh
  start_check readme-contract bash scripts/dev/test-readme-contract.sh
  start_check native-removal bash -c 'bun run test:native-removal && bun run check:native-removal'
  start_check linux-distribution bash -c 'bun run test:linux-artifact-identity && bun run test:linux-distribution-smoke'
  start_check workspace-rust-version bash -c 'bun run test:workspace-rust-version && bun run check:workspace-rust-version'
  start_check workspace-layout bash -c 'bun run test:workspace-layout && bun run check:workspace-layout'
  start_check pr-ci bash scripts/test-pr-ci.sh
  start_check repository-policy bun run test:repository-policy
  start_check pr-branch-freshness bun run test:pr-branch-freshness
  start_check security bash scripts/test-dependency-security.sh
  start_check bun-pin-fixtures bash scripts/test-bun-pin.sh
  start_check bun-pin-policy bash scripts/check-bun-toolchain-policy.sh

  for index in "${!check_pids[@]}"; do
    if ! wait "${check_pids[$index]}"; then
      status=1
      summarize_failure "${check_labels[$index]}" "${check_logs[$index]}"
    fi
  done
  return "$status"
}

run_with_budget 60 pre-commit run_checks

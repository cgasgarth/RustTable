#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

run_check() {
  local label="$1"
  shift
  local status=0
  if "$@" >"$temporary_directory/$label.log" 2>&1; then
    status=0
  else
    status="$?"
  fi
  return "$status"
}

cheap_labels=(diff fmt metadata source bun repository-policy)
run_check diff git diff --check &
cheap_pids[0]=$!
run_check fmt cargo fmt --all -- --check &
cheap_pids[1]=$!
run_check metadata cargo metadata --locked --no-deps --format-version 1 &
cheap_pids[2]=$!
run_check source bash scripts/check-source-policy.sh &
cheap_pids[3]=$!
run_check bun bun run test:computer-use &
cheap_pids[4]=$!
run_check repository-policy bun run test:repository-policy &
cheap_pids[5]=$!
if [[ "${RUSTTABLE_SKIP_PR_CI_REGRESSION:-0}" != 1 ]]; then
  cheap_labels+=(pr-ci)
  run_check pr-ci bash scripts/test-pr-ci.sh &
  cheap_pids[6]=$!
fi

cheap_status=0
for index in "${!cheap_pids[@]}"; do
  if ! wait "${cheap_pids[$index]}"; then
    cheap_status=1
    printf 'PR check failed: %s\n' "${cheap_labels[$index]}" >&2
    cat "$temporary_directory/${cheap_labels[$index]}.log" >&2
  fi
done
if (( cheap_status != 0 )); then
  exit "$cheap_status"
fi

if ! cargo clippy --workspace --all-targets --all-features --locked -- -D warnings \
  >"$temporary_directory/clippy.log" 2>&1; then
  cat "$temporary_directory/clippy.log" >&2
  exit 1
fi

if ! cargo test --workspace --all-targets --all-features --locked \
  >"$temporary_directory/test.log" 2>&1; then
  cat "$temporary_directory/test.log" >&2
  exit 1
fi

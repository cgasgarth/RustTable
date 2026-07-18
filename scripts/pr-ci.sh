#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

run_check() {
  local label="$1"
  shift
  if "$@" >"$temporary_directory/$label.log" 2>&1; then
    return 0
  fi
  return "$?"
}

cheap_labels=(diff fmt metadata source bun)
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

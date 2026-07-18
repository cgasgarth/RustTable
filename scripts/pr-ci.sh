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

cheap_labels=(diff fmt metadata source bun macos-artifact-identity repository-policy pr-branch-freshness doctor readme-contract native-removal linux-distribution)
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
run_check macos-artifact-identity bun run test:macos-artifact-identity &
cheap_pids[5]=$!
run_check repository-policy bun run test:repository-policy &
cheap_pids[6]=$!
run_check pr-branch-freshness bun run test:pr-branch-freshness &
cheap_pids[7]=$!
run_check doctor bash scripts/dev/test-doctor.sh &
cheap_pids[8]=$!
run_check readme-contract bash scripts/dev/test-readme-contract.sh &
cheap_pids[9]=$!
run_check native-removal bash -c 'bun run test:native-removal && bun run check:native-removal' &
cheap_pids[10]=$!
run_check linux-distribution bash -c 'bun run test:linux-artifact-identity && bun run test:linux-distribution-smoke' &
cheap_pids[11]=$!
cheap_index=12
if [[ "${RUSTTABLE_SKIP_BUN_PIN_REGRESSION:-0}" != 1 ]]; then
  cheap_labels+=(bun-pin-fixtures bun-pin-policy)
  run_check bun-pin-fixtures bash scripts/test-bun-pin.sh &
  cheap_pids[$cheap_index]=$!
  cheap_index=$((cheap_index + 1))
  run_check bun-pin-policy bash scripts/check-bun-toolchain-policy.sh &
  cheap_pids[$cheap_index]=$!
  cheap_index=$((cheap_index + 1))
fi
cheap_labels+=(workspace-rust-version)
run_check workspace-rust-version bash -c 'bun run test:workspace-rust-version && bun run check:workspace-rust-version' &
cheap_pids[$cheap_index]=$!
cheap_index=$((cheap_index + 1))
cheap_labels+=(workspace-layout)
run_check workspace-layout env RUSTTABLE_LAYOUT_CHECK=1 bash -c 'bun run test:workspace-layout && bun run check:workspace-layout' &
cheap_pids[$cheap_index]=$!
cheap_index=$((cheap_index + 1))
if [[ "${RUSTTABLE_SKIP_PR_CI_REGRESSION:-0}" != 1 ]]; then
  cheap_labels+=(pr-ci)
  run_check pr-ci bash scripts/test-pr-ci.sh &
  cheap_pids[$cheap_index]=$!
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

#!/usr/bin/env bash
set -euo pipefail

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

cargo fmt --all -- --check >"$temporary_directory/fmt.log" 2>&1 &
format_pid=$!
bash scripts/check-source-policy.sh >"$temporary_directory/policy.log" 2>&1 &
policy_pid=$!
bun run test:computer-use >"$temporary_directory/computer-use.log" 2>&1 &
computer_use_pid=$!

status=0
if ! wait "$format_pid"; then
  status=1
  cat "$temporary_directory/fmt.log" >&2
fi
if ! wait "$policy_pid"; then
  status=1
  cat "$temporary_directory/policy.log" >&2
fi
if ! wait "$computer_use_pid"; then
  status=1
  cat "$temporary_directory/computer-use.log" >&2
fi

exit "$status"

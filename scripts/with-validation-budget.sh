#!/usr/bin/env bash
set -euo pipefail

run_with_budget() {
  local limit="$1"
  shift
  local start_seconds="$SECONDS"
  local command_status=0
  if "$@"; then
    command_status=0
  else
    command_status=$?
  fi
  local elapsed_seconds=$((SECONDS - start_seconds))
  printf 'validation duration: %ss (budget: %ss)\n' "$elapsed_seconds" "$limit"
  if (( command_status != 0 )); then
    return "$command_status"
  fi
  if (( elapsed_seconds > limit )); then
    printf 'validation budget exceeded by %ss\n' "$((elapsed_seconds - limit))" >&2
    return 124
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  if (( $# < 2 )); then
    printf 'usage: %s LIMIT_SECONDS COMMAND [ARGUMENT ...]\n' "$0" >&2
    exit 2
  fi
  limit_seconds="$1"
  shift
  if [[ ! "$limit_seconds" =~ ^[0-9]+$ ]]; then
    printf 'validation budget must be a non-negative integer: %s\n' "$limit_seconds" >&2
    exit 2
  fi
  run_with_budget "$limit_seconds" "$@"
fi

#!/usr/bin/env bash
set -euo pipefail

budget_process_id=""

run_with_budget() {
  local limit="$1"
  local label="$2"
  shift
  shift
  local start_seconds="$SECONDS"
  local command_pid
  local elapsed_seconds

  if [[ ! "$limit" =~ ^[0-9]+$ ]]; then
    printf 'validation budget must be a non-negative integer: %s\n' "$limit" >&2
    return 2
  fi
  if [[ -z "$label" ]]; then
    printf 'validation label must not be empty\n' >&2
    return 2
  fi

  "$@" &
  command_pid=$!
  budget_process_id="$command_pid"

  while kill -0 "$command_pid" 2>/dev/null; do
    elapsed_seconds=$((SECONDS - start_seconds))
    if (( elapsed_seconds >= limit )); then
      terminate_process_tree "$command_pid"
      wait "$command_pid" 2>/dev/null || true
      budget_process_id=""
      printf 'validation duration: %ss (budget: %ss, label: %s)\n' "$elapsed_seconds" "$limit" "$label"
      printf 'validation budget exceeded\n' >&2
      return 124
    fi
    sleep 1
  done

  local command_status=0
  if wait "$command_pid"; then
    command_status=0
  else
    command_status=$?
  fi
  budget_process_id=""
  elapsed_seconds=$((SECONDS - start_seconds))
  printf 'validation duration: %ss (budget: %ss, label: %s)\n' "$elapsed_seconds" "$limit" "$label"
  if (( command_status != 0 )); then
    return "$command_status"
  fi
  if (( elapsed_seconds > limit )); then
    printf 'validation budget exceeded by %ss\n' "$((elapsed_seconds - limit))" >&2
    return 124
  fi
}

terminate_process_tree() {
  local process_id="$1"
  local child_id
  local child_ids

  child_ids="$(pgrep -P "$process_id" 2>/dev/null || true)"
  for child_id in $child_ids; do
    terminate_process_tree "$child_id"
  done
  kill -TERM "$process_id" 2>/dev/null || true
}

budget_signal() {
  local status="$1"
  if [[ -n "$budget_process_id" ]]; then
    terminate_process_tree "$budget_process_id"
    wait "$budget_process_id" 2>/dev/null || true
    budget_process_id=""
  fi
  exit "$status"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  if (( $# < 3 )); then
    printf 'usage: %s LIMIT_SECONDS LABEL COMMAND [ARGUMENT ...]\n' "$0" >&2
    exit 2
  fi
  limit_seconds="$1"
  shift
  label="$1"
  shift
  trap 'budget_signal 130' INT
  trap 'budget_signal 143' TERM
  command_status=0
  if run_with_budget "$limit_seconds" "$label" "$@"; then
    command_status=0
  else
    command_status=$?
  fi
  trap - INT TERM
  exit "$command_status"
fi

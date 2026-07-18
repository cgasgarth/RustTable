#!/usr/bin/env bash
set -euo pipefail

helper="$(dirname "$0")/with-validation-budget.sh"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

bash "$helper" 5 success bash -c ':' >/dev/null

if bash "$helper" 5 status bash -c 'exit 7' >/dev/null 2>&1; then
  printf 'expected a nonzero command status to propagate\n' >&2
  exit 1
elif [[ "$?" -ne 7 ]]; then
  printf 'expected status 7 to propagate\n' >&2
  exit 1
fi

argument_file="$temporary_directory/arguments"
bash "$helper" 5 arguments bash -c 'printf "%s" "$1" > "$2"' _ 'value with spaces; $HOME' "$argument_file" >/dev/null
if [[ "$(<"$argument_file")" != 'value with spaces; $HOME' ]]; then
  printf 'argument preservation regression\n' >&2
  exit 1
fi

deadline_marker="$temporary_directory/deadline-marker"
if bash "$helper" 0 deadline bash -c 'sleep 2; touch "$1"' _ "$deadline_marker" >/dev/null 2>&1; then
  printf 'expected an over-budget command to fail\n' >&2
  exit 1
fi
sleep 1
if [[ -e "$deadline_marker" ]]; then
  printf 'deadline command outlived its process tree\n' >&2
  exit 1
fi

for signal in INT TERM; do
  signal_marker="$temporary_directory/$signal-marker"
  if bash -c '
    source "$1"
    trap "budget_signal 130" INT
    trap "budget_signal 143" TERM
    (sleep 1; kill -"$2" "$$") &
    sender_pid=$!
    command_status=0
    if run_with_budget 10 "$2" bash -c '\''sleep 3; touch "$1"'\'' _ "$3"; then
      command_status=0
    else
      command_status=$?
    fi
    wait "$sender_pid" 2>/dev/null || true
    exit "$command_status"
  ' _ "$helper" "$signal" "$signal_marker" >/dev/null 2>&1; then
    runner_pid=0
  else
    runner_pid=$?
  fi
  if [[ "$runner_pid" -eq 0 ]]; then
    printf '%s should terminate the helper\n' "$signal" >&2
    exit 1
  fi
  sleep 1
  if [[ -e "$signal_marker" ]]; then
    printf '%s command outlived its process tree\n' "$signal" >&2
    exit 1
  fi
done

printf 'validation budget regression tests passed\n'

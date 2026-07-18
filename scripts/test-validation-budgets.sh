#!/usr/bin/env bash
set -euo pipefail

helper="$(dirname "$0")/with-validation-budget.sh"

bash "$helper" 1 bash -c ':' >/dev/null
if bash "$helper" 0 bash -c 'sleep 1' >/dev/null 2>&1; then
  printf 'expected an over-budget command to fail\n' >&2
  exit 1
fi

printf 'validation budget regression tests passed\n'

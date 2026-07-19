#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

cat >"$temporary_directory/cargo" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"${FAKE_LOG:?}"
if [ "${FAKE_FAIL:-0}" = 1 ]; then
  exit 17
fi
EOF
chmod +x "$temporary_directory/cargo"

FAKE_LOG="$temporary_directory/log"
export FAKE_LOG

PATH="$temporary_directory:$PATH" /bin/bash "$root_directory/scripts/precommit-fast.sh"
grep -Fx 'xtask ci precommit' "$FAKE_LOG" >/dev/null

if PATH="$temporary_directory:$PATH" FAKE_FAIL=1 \
  /bin/bash "$root_directory/scripts/precommit-fast.sh"; then
  echo 'expected precommit failure to propagate' >&2
  exit 1
fi

printf 'pre-commit fast delegation fixtures passed\n'

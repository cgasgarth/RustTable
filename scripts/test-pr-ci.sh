#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

if grep -nE 'verify-pr-contract|GITHUB_EVENT_PATH' "$root_directory/.github/workflows/rust-pr.yml"; then
  echo 'PR validation must not enforce issue linkage or body structure' >&2
  exit 1
fi
if grep -nF 'cargo xtask repo verify-workflows' "$root_directory/.github/workflows/rust-pr.yml"; then
  echo 'PR validation must not build xtask twice for workflow inventory' >&2
  exit 1
fi
grep -Fx 'pull_request = 150' "$root_directory/quality/validation-surfaces.toml" >/dev/null

cat >"$temporary_directory/cargo" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"${FAKE_LOG:?}"
printf '%s\n' "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-}" "CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-}" "CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-}" "CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-}" >"${FAKE_ENV_LOG:?}"
if [ "${FAKE_FAIL:-0}" = 1 ]; then
  exit 19
fi
EOF
chmod +x "$temporary_directory/cargo"

FAKE_LOG="$temporary_directory/log"
FAKE_ENV_LOG="$temporary_directory/env"
export FAKE_LOG
export FAKE_ENV_LOG

PATH="$temporary_directory:$PATH" /bin/bash "$root_directory/scripts/pr-ci.sh"
grep -Fx 'xtask ci pr' "$FAKE_LOG" >/dev/null
grep -Fx 'CARGO_INCREMENTAL=0' "$FAKE_ENV_LOG" >/dev/null
grep -Fx 'CARGO_BUILD_JOBS=2' "$FAKE_ENV_LOG" >/dev/null
grep -Fx 'CARGO_PROFILE_DEV_DEBUG=0' "$FAKE_ENV_LOG" >/dev/null
grep -Fx 'CARGO_PROFILE_TEST_DEBUG=0' "$FAKE_ENV_LOG" >/dev/null

if PATH="$temporary_directory:$PATH" FAKE_FAIL=1 \
  /bin/bash "$root_directory/scripts/pr-ci.sh"; then
  echo 'expected PR validation failure to propagate' >&2
  exit 1
fi

printf 'PR validation delegation fixtures passed\n'

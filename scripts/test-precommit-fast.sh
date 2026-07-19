#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

write_fake_tools() {
  local directory="$1"
  cat >"$directory/git" <<'EOF'
#!/bin/sh
if [ "${FAKE_FAILS:-}" = diff ]; then
  echo "fake diff failure"
  exit 11
fi
exit 0
EOF
  cat >"$directory/cargo" <<'EOF'
#!/bin/sh
case "${1:-}" in
  fmt) label=fmt ;;
  metadata) label=metadata ;;
  check) label=rust-check ;;
  clippy) label=rust-clippy ;;
  test) label=rust-test ;;
  xtask) label=workflow-policy ;;
  *) label=other ;;
esac
case ",${FAKE_FAILS:-}," in
  *",$label,"*)
    echo "fake $label failure"
    exit 12
    ;;
esac
if [ "${FAKE_SLEEP:-0}" != 0 ]; then
  sleep "$FAKE_SLEEP"
fi
if [ "$label" = rust-check ] || [ "$label" = rust-clippy ] || [ "$label" = rust-test ]; then
  printf '%s\n' "$label" >>"${FAKE_MARKERS:?}"
fi
exit 0
EOF
  cat >"$directory/bun" <<'EOF'
#!/bin/sh
label=other
case "$*" in
  *test:computer-use*) label=computer-use ;;
  *test:macos-artifact-identity*) label=macos-artifact-identity ;;
  *test:native-removal*|*check:native-removal*) label=native-removal ;;
  *test:linux-artifact-identity*|*test:linux-distribution-smoke*) label=linux-distribution ;;
  *test:workspace-rust-version*|*check:workspace-rust-version*) label=workspace-rust-version ;;
  *test:workspace-layout*|*check:workspace-layout*) label=workspace-layout ;;
  *test:cache-workflow-policy*|*check:cache-workflow-policy*) label=cache-workflow-policy ;;
  *test:repository-policy*) label=repository-policy ;;
  *test:pr-branch-freshness*) label=pr-branch-freshness ;;
esac
case ",${FAKE_FAILS:-}," in
  *",$label,"*)
    echo "fake $label failure"
    exit 13
    ;;
esac
if [ "${FAKE_SLEEP:-0}" != 0 ]; then
  sleep "$FAKE_SLEEP"
fi
exit 0
EOF
  cat >"$directory/bash" <<'EOF'
#!/bin/sh
case "${1:-}" in
  scripts/check-source-policy.sh) label=source ;;
  scripts/dev/test-doctor.sh) label=doctor ;;
  scripts/dev/test-readme-contract.sh) label=readme-contract ;;
  scripts/test-pr-ci.sh) label=pr-ci ;;
  scripts/test-dependency-security.sh) label=security ;;
  scripts/test-bun-pin.sh) label=bun-pin-fixtures ;;
  scripts/check-bun-toolchain-policy.sh) label=bun-pin-policy ;;
  *) label=other ;;
esac
case ",${FAKE_FAILS:-}," in
  *",$label,"*)
    echo "fake $label failure"
    exit 14
    ;;
esac
if [ "${FAKE_SLEEP:-0}" != 0 ] && [ "$label" != other ]; then
  sleep "$FAKE_SLEEP"
fi
if [ "$label" != other ]; then
  exit 0
fi
exec /bin/bash "$@"
EOF
  chmod +x "$directory"/{git,cargo,bun,bash}
}

fake_tools="$temporary_directory/tools"
mkdir -p "$fake_tools"
write_fake_tools "$fake_tools"

run_precommit() {
  local failures="$1"
  local output="$2"
  local status=0
  : >"$FAKE_MARKERS"
  if FAKE_FAILS="$failures" PATH="$fake_tools:$PATH" \
    /bin/bash "$root_directory/scripts/precommit-fast.sh" >"$output" 2>&1; then
    status=0
  else
    status="$?"
  fi
  return "$status"
}

FAKE_MARKERS="$temporary_directory/markers"
export FAKE_MARKERS

output="$temporary_directory/clean.log"
if ! run_precommit "" "$output"; then
  cat "$output" >&2
  exit 1
fi
for label in rust-check rust-clippy rust-test; do
  grep -Fx "$label" "$FAKE_MARKERS" >/dev/null
done
grep -F 'validation duration:' "$output" >/dev/null
grep -F 'budget: 60s' "$output" >/dev/null

output="$temporary_directory/failures.log"
if run_precommit 'rust-check,rust-clippy,source,cache-workflow-policy,workflow-policy' "$output"; then
  echo 'expected independent pre-commit failures' >&2
  exit 1
fi
for label in rust-check rust-clippy source cache-workflow-policy workflow-policy; do
  grep -F "pre-commit check failed: $label" "$output" >/dev/null
  grep -F "fake $label failure" "$output" >/dev/null
done
grep -Fx rust-test "$FAKE_MARKERS" >/dev/null

output="$temporary_directory/parallel.log"
start_seconds="$SECONDS"
if ! FAKE_SLEEP=2 run_precommit '' "$output"; then
  cat "$output" >&2
  exit 1
fi
elapsed_seconds=$((SECONDS - start_seconds))
if (( elapsed_seconds >= 10 )); then
  printf 'pre-commit fixture checks did not remain parallel: %ss\n' "$elapsed_seconds" >&2
  exit 1
fi

timeout_marker="$temporary_directory/timeout-marker"
if /bin/bash "$root_directory/scripts/with-validation-budget.sh" 0 precommit-fixture-timeout \
  /bin/bash -c 'sleep 2; touch "$1"' _ "$timeout_marker" >"$temporary_directory/timeout.log" 2>&1; then
  echo 'expected the pre-commit fixture timeout to fail' >&2
  exit 1
fi
sleep 1
if [[ -e "$timeout_marker" ]]; then
  echo 'timed-out pre-commit fixture left a child process running' >&2
  exit 1
fi

printf 'pre-commit fast fixtures passed\n'

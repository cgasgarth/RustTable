#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
script="$root/scripts/dev/precommit-check.sh"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/bin" "$fixture/repo"
cat >"$fixture/bin/git" <<'EOF'
#!/usr/bin/env bash
if [[ "$*" == "rev-parse --show-toplevel" ]]; then
  printf '%s\n' "$FAKE_ROOT"
  exit 0
fi
exit 1
EOF
cat >"$fixture/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${CARGO_BUILD_JOBS-}|$*" >"$FAKE_LOG"
if [[ "${FAKE_FAIL:-0}" == 1 ]]; then
  for line in $(seq 1 100); do
    printf 'failure-line-%s\n' "$line"
  done
  exit 7
fi
printf 'routine success output\n'
EOF
chmod +x "$fixture/bin/git" "$fixture/bin/cargo"

success_output="$(CARGO_BUILD_JOBS=10 FAKE_ROOT="$fixture/repo" FAKE_LOG="$fixture/log" PATH="$fixture/bin:$PATH" bash "$script")"
[[ "$(<"$fixture/log")" == '|xtask check --parallel' ]]
[[ "$success_output" == 'PASS pre-commit check (Cargo host parallelism, cargo-owner=1)' ]]

set +e
failure_output="$(FAKE_FAIL=1 FAKE_ROOT="$fixture/repo" FAKE_LOG="$fixture/failure-log" PATH="$fixture/bin:$PATH" bash "$script" 2>&1)"
failure_status=$?
set -e
[[ "$failure_status" -eq 7 ]]
[[ "$failure_output" == *'failure-line-100'* ]]
[[ "$failure_output" != *'routine success output'* ]]
[[ "$(printf '%s\n' "$failure_output" | wc -l)" -le 82 ]]

printf 'pre-commit-check: tests PASS\n'

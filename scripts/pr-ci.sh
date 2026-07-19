#!/usr/bin/env bash
set -euo pipefail

# The PR lane is intentionally strict. GitHub Actions can invoke one contract
# group per job so independent repository and Rust checks overlap on runners.
# Avoid CI-only debug and incremental state overhead without changing commands,
# features, targets, or warning policy.
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
# The GitHub Linux runner has multiple cores; keep a sixteen-job floor because
# ubuntu-latest can report only a small number of schedulable CPUs while the
# cold Wasmtime test lane is compiling. An explicit caller override remains
# authoritative.
reported_jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '1')"
if [[ "$reported_jobs" =~ ^[0-9]+$ ]] && (( reported_jobs < 16 )); then
  reported_jobs=16
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$reported_jobs}"

if [[ -n "${RUSTTABLE_CI_GROUP:-}" ]]; then
  exec cargo xtask ci pr --group "$RUSTTABLE_CI_GROUP"
fi

exec cargo xtask ci pr

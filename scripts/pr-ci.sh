#!/usr/bin/env bash
set -euo pipefail

# The PR lane is intentionally strict, but a cold runner must compile the
# workspace three times (check, clippy, and test) inside the 150-second budget.
# Avoid CI-only debug and incremental state overhead without changing commands,
# features, targets, or warning policy.
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
# The GitHub Linux runner has multiple cores; use its reported capacity so the
# cold test/clippy/check lanes finish inside the strict PR wall-clock budget.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '1')}"

exec cargo xtask ci pr

#!/usr/bin/env bash
set -euo pipefail

# The PR lane is intentionally strict, but a cold runner must compile the
# workspace three times (check, clippy, and test) inside the 150-second budget.
# Avoid CI-only debug and incremental state overhead without changing commands,
# features, targets, or warning policy.
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
# Three Cargo processes are intentionally run in parallel by the validation
# contract. Keep each process bounded so cold dependency builds share the
# runner instead of multiplying compiler fan-out.
export CARGO_BUILD_JOBS=2

exec cargo xtask ci pr

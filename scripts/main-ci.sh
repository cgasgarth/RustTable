#!/usr/bin/env bash
set -euo pipefail

run_step() {
  local label="$1"
  shift
  printf 'main validation: %s\n' "$label"
  "$@"
}

run_step fast-gate bash scripts/pr-ci.sh
run_step doctests cargo test --workspace --doc --all-features --locked
run_step release-tests cargo test --workspace --all-targets --all-features --release --locked
run_step release-build cargo build --workspace --all-targets --all-features --release --locked
run_step documentation env RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked

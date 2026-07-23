#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
justfile="$root/justfile"

recipe() {
  awk -v name="$1" '
    $0 == name ":" { found = 1; next }
    found && $0 !~ /^[[:space:]]/ { exit }
    found && $0 ~ /^[[:space:]]/ {
      sub(/^[[:space:]]+/, "")
      print
    }
  ' "$justfile"
}

assert_recipe() {
  local name="$1"
  local expected="$2"
  local actual
  actual="$(recipe "$name")"
  [[ "$actual" == "$expected" ]] || {
    printf 'justfile: %s recipe changed: %s\n' "$name" "$actual" >&2
    exit 1
  }
}

assert_recipe fmt 'cargo fmt --all'
assert_recipe check 'cargo xtask check'
assert_recipe test 'cargo test --workspace --all-targets --all-features --locked'
assert_recipe ci 'bash scripts/dev/precommit-check.sh'
assert_recipe run 'cargo run --package rusttable-app --bin rusttable-app --locked'

grep -Fqx 'unset CARGO_BUILD_JOBS' "$root/scripts/dev/precommit-check.sh"
grep -Fqx 'cargo xtask check --parallel >"$log" 2>&1' "$root/scripts/dev/precommit-check.sh"
! grep -Eq '^[[:space:]]*jobs[[:space:]]*=' "$root/.cargo/config.toml"
for script in \
  scripts/dev/precommit-check.sh \
  scripts/dev/install-computer-use-app.ts \
  scripts/linux-distribution-smoke.sh \
  scripts/macos-distribution-smoke.sh; do
  ! grep -Fq 'CARGO_BUILD_JOBS=10' "$root/$script"
done

printf 'justfile contract: PASS\n'

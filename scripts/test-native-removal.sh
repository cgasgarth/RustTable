#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
checker="$root/scripts/check-source-policy.sh"

bash "$checker" >/dev/null

fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
git -C "$fixture" init -q
mkdir -p "$fixture/scripts" "$fixture/crates/rusttable-fixture/src"
cp "$checker" "$fixture/scripts/check-source-policy.sh"
printf '[workspace]\n' >"$fixture/Cargo.toml"
printf 'fn main() {}\n' >"$fixture/crates/rusttable-fixture/src/lib.rs"
git -C "$fixture" add .
printf 'legacy native payload\n' >"$fixture/crates/rusttable-fixture/src/legacy.c"

if (cd "$fixture" && bash scripts/check-source-policy.sh) >/dev/null 2>&1; then
  printf 'source-policy: fixture unexpectedly passed\n' >&2
  exit 1
fi

printf 'source-policy: tests PASS\n'

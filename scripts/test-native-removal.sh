#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
checker="$root/scripts/check-native-removal.sh"

bash "$checker" >/dev/null

fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
git -C "$fixture" init -q
mkdir -p "$fixture/scripts" "$fixture/crates/rusttable-fixture/src"
cp "$checker" "$fixture/scripts/check-native-removal.sh"
printf '[workspace]\n' >"$fixture/Cargo.toml"
printf 'int legacy(void) { return 0; }\n' >"$fixture/crates/rusttable-fixture/src/legacy.c"
printf '%s\n' 'cmake -S . -B build' >"$fixture/scripts/legacy-build.sh"
git -C "$fixture" add .

if (cd "$fixture" && bash scripts/check-native-removal.sh) >/dev/null 2>&1; then
  printf 'native-removal: fixture unexpectedly passed\n' >&2
  exit 1
fi

printf 'native-removal: tests PASS\n'

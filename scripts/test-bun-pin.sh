#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd -P)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

assert_contains() {
  local needle="$1"
  local haystack="$2"
  if [[ "$haystack" != *"$needle"* ]]; then
    printf 'expected output to contain %s, got:\n%s\n' "$needle" "$haystack" >&2
    exit 1
  fi
}

assert_empty() {
  local value="$1"
  if [[ -n "$value" ]]; then
    printf 'expected empty output, got:\n%s\n' "$value" >&2
    exit 1
  fi
}

make_root() {
  local name="$1"
  local package_manager="$2"
  local fixture="$temporary_directory/$name"
  mkdir -p "$fixture/.github/workflows"
  printf '%s\n' "{\"name\":\"fixture\",\"packageManager\":$package_manager}" >"$fixture/package.json"
  printf '%s\n' "$(valid_workflow)" >"$fixture/.github/workflows/rust-main.yml"
  printf '%s\n' "$fixture"
}

valid_workflow() {
  cat <<'EOF'
name: Fixture
on:
  push:
    branches: [main]
permissions:
  contents: read
jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - name: Check out repository
        uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683
      - name: Read canonical Bun pin
        id: bun-pin
        run: bash scripts/bun-pin.sh "$GITHUB_WORKSPACE" >> "$GITHUB_OUTPUT"
      - name: Set up exact Bun toolchain
        uses: oven-sh/setup-bun@735343b667d3e6f658f44d0eca948eb6282f2b76
        with:
          bun-version: ${{ steps.bun-pin.outputs.bun-version }}
      - name: Verify canonical Bun version
        run: test "$(bun --version)" = "${{ steps.bun-pin.outputs.bun-version }}"
      - name: Restore Cargo cache
        uses: actions/cache@5a3ec84eff668545956fd18022155c47e93e2684
        with:
          path: target
          key: fixture-${{ runner.os }}-rust-beta-2026-07-17-${{ hashFiles('Cargo.lock', 'package.json') }}
      - name: Run Bun command
        run: bun run scripts/example.ts
EOF
}

insert_duplicate_pin_step() {
  local workflow="$1"
  awk '{
    print
    if ($0 ~ /- name: Set up exact Bun toolchain/) {
      print "      - name: Read canonical Bun pin again"
      print "        id: bun-pin-again"
      print "        run: bash scripts/bun-pin.sh \"$GITHUB_WORKSPACE\" >> \"$GITHUB_OUTPUT\""
    }
  }' "$workflow" >"$workflow.next"
  mv "$workflow.next" "$workflow"
}

run_pin() {
  local fixture="$1"
  local output_file="$2"
  local error_file="$3"
  if bash "$root_directory/scripts/bun-pin.sh" "$fixture" >"$output_file" 2>"$error_file"; then
    return 0
  else
    return "$?"
  fi
}

valid_fixture="$(make_root valid '"bun@1.3.14"')"
output_file="$temporary_directory/valid-pin.log"
run_pin "$valid_fixture" "$output_file" "$temporary_directory/valid-pin.err"
[[ "$(cat "$output_file")" == 'bun-version=1.3.14' ]]

for case_name in malformed missing duplicate wrong-manager; do
  case "$case_name" in
    malformed) package_json='{ "packageManager": "bun@1.3.14"';;
    missing) package_json='{ "name": "fixture" }';;
    duplicate) package_json='{ "packageManager": "bun@1.3.14", "packageManager": "bun@1.3.14" }';;
    wrong-manager) package_json='{ "packageManager": "npm@10.0.0" }';;
  esac
  fixture="$temporary_directory/pin-$case_name"
  mkdir -p "$fixture"
  printf '%s\n' "$package_json" >"$fixture/package.json"
  output_file="$temporary_directory/$case_name.log"
  error_file="$temporary_directory/$case_name.err"
  if run_pin "$fixture" "$output_file" "$error_file"; then
    printf 'expected %s package fixture to fail\n' "$case_name" >&2
    exit 1
  fi
  assert_empty "$(cat "$output_file")"
  if [[ "$case_name" == malformed ]]; then
    assert_contains 'malformed JSON' "$(cat "$error_file")"
  else
    assert_contains 'packageManager' "$(cat "$error_file")"
  fi
done

injection_value='1.3.14'
injection_value="${injection_value}\$(touch /tmp/bun-pin-injection)"
for value in latest '^1.3.14' '>=1.3.14' '1.x' '1.3.14-beta' '1.3.14+build' "$injection_value"; do
  fixture="$temporary_directory/pin-invalid-${value//[^A-Za-z0-9]/_}"
  mkdir -p "$fixture"
  printf '{ "packageManager": "bun@%s" }\n' "$value" >"$fixture/package.json"
  output_file="$temporary_directory/invalid.log"
  if run_pin "$fixture" "$output_file" "$temporary_directory/invalid.err"; then
    printf 'expected invalid Bun version fixture to fail: %s\n' "$value" >&2
    exit 1
  fi
  assert_empty "$(sed -n '/^bun-version=/p' "$output_file")"
done

control_fixture="$temporary_directory/pin-control"
mkdir -p "$control_fixture"
control_value='bun@1.3.14
GITHUB_OUTPUT=poisoned'
printf '{ "packageManager": "%s" }\n' "$control_value" >"$control_fixture/package.json"
if run_pin "$control_fixture" "$temporary_directory/control.log" "$temporary_directory/control.err"; then
  printf 'expected control-character package fixture to fail\n' >&2
  exit 1
fi
assert_empty "$(cat "$temporary_directory/control.log")"
assert_contains 'control character' "$(cat "$temporary_directory/control.err")"

policy_fixture="$valid_fixture"
if ! bash "$root_directory/scripts/check-bun-toolchain-policy.sh" "$policy_fixture"; then
  printf 'expected compliant workflow fixture to pass\n' >&2
  exit 1
fi

policy_cases=(literal-drift missing-verification wrong-order stale-cache duplicate-pin)
for case_name in "${policy_cases[@]}"; do
  fixture="$temporary_directory/policy-$case_name"
  cp -R "$valid_fixture" "$fixture"
  workflow="$fixture/.github/workflows/rust-main.yml"
  case "$case_name" in
    literal-drift)
      sed -i.bak 's/${{ steps.bun-pin.outputs.bun-version }}/1.3.14/g' "$workflow"
      ;;
    missing-verification)
      sed -i.bak '/Verify canonical Bun version/,+1d' "$workflow"
      ;;
    wrong-order)
      sed -i.bak '/Read canonical Bun pin/,+2d' "$workflow"
      sed -i.bak '/Set up exact Bun toolchain/,+3d' "$workflow"
      sed -i.bak '/Verify canonical Bun version/,+1d' "$workflow"
      ;;
    stale-cache)
      sed -i.bak "s/, 'package.json'//" "$workflow"
      ;;
    duplicate-pin)
      insert_duplicate_pin_step "$workflow"
      ;;
  esac
  if bash "$root_directory/scripts/check-bun-toolchain-policy.sh" "$fixture" >"$temporary_directory/policy.log" 2>&1; then
    printf 'expected policy fixture to fail: %s\n' "$case_name" >&2
    exit 1
  fi
  case "$case_name" in
    literal-drift) assert_contains 'Bun version selection must use canonical output' "$(cat "$temporary_directory/policy.log")";;
    missing-verification) assert_contains 'verification step is missing' "$(cat "$temporary_directory/policy.log")";;
    wrong-order) assert_contains 'Bun setup sequence is incomplete or out of order' "$(cat "$temporary_directory/policy.log")";;
    stale-cache) assert_contains 'cache key must hash package.json' "$(cat "$temporary_directory/policy.log")";;
    duplicate-pin) assert_contains 'canonical pin is read 2 times' "$(cat "$temporary_directory/policy.log")";;
  esac
done

multi_fixture="$temporary_directory/policy-multiple"
cp -R "$valid_fixture" "$multi_fixture"
multi_workflow="$multi_fixture/.github/workflows/rust-main.yml"
sed -i.bak 's/bun-version: .*/bun-version: latest/' "$multi_workflow"
sed -i.bak '/Verify canonical Bun version/,+1d' "$multi_workflow"
sed -i.bak "s/, 'package.json'//" "$multi_workflow"
insert_duplicate_pin_step "$multi_workflow"
if bash "$root_directory/scripts/check-bun-toolchain-policy.sh" "$multi_fixture" >"$temporary_directory/multiple.log" 2>&1; then
  printf 'expected simultaneous policy fixture to fail\n' >&2
  exit 1
fi
multiple_output="$(cat "$temporary_directory/multiple.log")"
assert_contains 'canonical pin is read 2 times' "$multiple_output"
assert_contains 'Bun version selection must use canonical output' "$multiple_output"
assert_contains 'verification step is missing' "$multiple_output"
assert_contains 'cache key must hash package.json' "$multiple_output"
[[ "$(printf '%s\n' "$multiple_output" | grep -c 'workflow .*job validate')" -ge 2 ]]

printf 'bun pin and hosted policy fixtures passed\n'

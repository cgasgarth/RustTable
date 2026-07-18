#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source_root="$(cd -- "$script_directory/.." && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

passed=0
failed=0

assert_contains() {
  local file="$1"
  local expected="$2"
  rg -F -- "$expected" "$file" >/dev/null || {
    printf 'missing expected text %q in %s\n' "$expected" "$file" >&2
    return 1
  }
}

prepare_fixture() {
  local name="$1"
  local root="$temporary_directory/$name"
  mkdir -p "$root/scripts" "$root/bin" "$root/target"
  cp "$source_root/scripts/dependency-security.sh" "$root/scripts/dependency-security.sh"
  cp "$source_root/scripts/validate-package-manifest.ts" "$root/scripts/validate-package-manifest.ts"
  cp "$source_root/scripts/security-tool-versions.env" "$root/scripts/security-tool-versions.env"
  cp "$source_root/deny.toml" "$root/deny.toml"
  cp "$source_root/Cargo.lock" "$root/Cargo.lock"
  cp "$source_root/Cargo.toml" "$root/Cargo.toml"
  cp "$source_root/package.json" "$root/package.json"
  git -C "$root" init -q
  git -C "$root" config user.email test@example.invalid
  git -C "$root" config user.name test
  git -C "$root" add Cargo.lock Cargo.toml deny.toml package.json scripts
  git -C "$root" commit -qm fixture
  printf '%s' "$root"
}

write_fake_tools() {
  local root="$1"
  local behavior="${2:-success}"
  cat > "$root/bin/cargo-audit" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "cargo-audit 0.22.2" > "\${RUSTTABLE_SECURITY_TRACE:?}/audit-version"
if [[ "\$1" == "--version" ]]; then [[ "$behavior" == audit-version-mismatch ]] && printf 'cargo-audit 0.22.1\n' || printf 'cargo-audit 0.22.2\n'; exit 0; fi
printf '%s\n' "audit \$*" >> "\${RUSTTABLE_SECURITY_TRACE:?}/stages"
[[ "$behavior" != audit-failure ]]
EOF
  cat > "$root/bin/cargo-deny" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "cargo-deny 0.19.8" > "\${RUSTTABLE_SECURITY_TRACE:?}/deny-version"
if [[ "\$1" == "--version" ]]; then [[ "$behavior" == deny-version-mismatch ]] && printf 'cargo-deny 0.19.7\n' || printf 'cargo-deny 0.19.8\n'; exit 0; fi
printf '%s\n' "deny \$*" >> "\${RUSTTABLE_SECURITY_TRACE:?}/stages"
[[ "$behavior" != deny-failure ]]
EOF
  cat > "$root/bin/cargo" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "cargo \$*" >> "\${RUSTTABLE_SECURITY_TRACE:?}/stages"
case "\${1:-}" in
  metadata) [[ "$behavior" != metadata-failure ]];;
  tree) [[ "$behavior" != duplicate-failure ]];;
  *) exit 2;;
esac
EOF
  cat > "$root/bin/bun" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "--version" ]]; then
  [[ "$behavior" != bun-version-mismatch ]] && printf '1.3.14\n' || printf '1.3.13\n'
  exit 0
fi
printf '%s\n' "bun \$*" >> "\${RUSTTABLE_SECURITY_TRACE:?}/stages"
[[ "$behavior" != javascript-failure ]]
EOF
  chmod +x "$root/bin"/*
}

run_case() {
  local name="$1"
  local behavior="$2"
  local expected_status="$3"
  local expected_stage="$4"
  local root
  root="$(prepare_fixture "$name")"
  mkdir -p "$root/trace"
  write_fake_tools "$root" "$behavior"
  local output="$root/output.log"
  local actual_status=0
  RUSTTABLE_SECURITY_TRACE="$root/trace" \
    CARGO_BIN="$root/bin/cargo" \
    CARGO_AUDIT_BIN="$root/bin/cargo-audit" \
    CARGO_DENY_BIN="$root/bin/cargo-deny" \
    BUN_BIN="$root/bin/bun" \
    bash "$root/scripts/dependency-security.sh" > "$output" 2>&1 || actual_status=$?
  if [[ "$actual_status" != "$expected_status" ]]; then
    printf '%s: expected status %s, got %s\n' "$name" "$expected_status" "$actual_status" >&2
    sed -n '1,80p' "$output" >&2
    return 1
  fi
  if [[ "$expected_stage" != none ]]; then
    assert_contains "$root/target/security/summary.txt" "failed_stage=$expected_stage"
  fi
  if [[ "$behavior" == success ]]; then
    assert_contains "$root/trace/stages" 'cargo metadata --locked --format-version 1'
    assert_contains "$root/trace/stages" 'audit audit --file'
    assert_contains "$root/trace/stages" 'deny check bans licenses sources'
    assert_contains "$root/trace/stages" 'cargo tree --workspace --all-features --duplicates --locked'
  fi
}

run_case success success 0 none && ((passed += 1))
run_case metadata_failure metadata-failure 1 metadata && ((passed += 1))
run_case audit_failure audit-failure 1 audit && ((passed += 1))
run_case deny_failure deny-failure 1 deny && ((passed += 1))
run_case duplicate_failure duplicate-failure 1 duplicates && ((passed += 1))
run_case audit_version_mismatch audit-version-mismatch 1 audit-version && ((passed += 1))
run_case deny_version_mismatch deny-version-mismatch 1 deny-version && ((passed += 1))
run_case bun_version_mismatch bun-version-mismatch 1 bun-version && ((passed += 1))
run_case javascript_failure javascript-failure 1 javascript-manifest && ((passed += 1))

missing_root="$(prepare_fixture missing_required_file)"
git -C "$missing_root" rm -q deny.toml
if (cd "$missing_root" && bash scripts/dependency-security.sh >output.log 2>&1); then
  printf 'missing required file case unexpectedly passed\n' >&2
  exit 1
fi
assert_contains "$missing_root/output.log" 'required tracked file is missing: deny.toml'
((passed += 1))

malformed_root="$(prepare_fixture malformed_tool_version)"
printf 'CARGO_AUDIT_VERSION=bad\nCARGO_DENY_VERSION=0.19.8\n' > "$malformed_root/scripts/security-tool-versions.env"
git -C "$malformed_root" add scripts/security-tool-versions.env
git -C "$malformed_root" commit -qm malformed
if (cd "$malformed_root" && bash scripts/dependency-security.sh >output.log 2>&1); then
  printf 'malformed version case unexpectedly passed\n' >&2
  exit 1
fi
assert_contains "$malformed_root/output.log" 'malformed version for CARGO_AUDIT_VERSION'
((passed += 1))

untracked_root="$(prepare_fixture untracked_required_file)"
git -C "$untracked_root" rm --cached -q deny.toml
if (cd "$untracked_root" && bash scripts/dependency-security.sh >output.log 2>&1); then
  printf 'untracked required file case unexpectedly passed\n' >&2
  exit 1
fi
assert_contains "$untracked_root/output.log" 'required tracked file is missing: deny.toml'
((passed += 1))

for section in dependencies devDependencies optionalDependencies peerDependencies; do
  package_root="$(prepare_fixture "package_$section")"
  bun -e 'const path = Bun.argv[1]; const value = JSON.parse(await Bun.file(path).text()); value[Bun.argv[2]] = { example: "1.0.0" }; await Bun.write(path, JSON.stringify(value));' "$package_root/package.json" "$section"
  git -C "$package_root" add package.json
  git -C "$package_root" commit -qm "prohibited $section"
  mkdir -p "$package_root/trace"
  write_fake_tools "$package_root" success
  if RUSTTABLE_SECURITY_TRACE="$package_root/trace" CARGO_BIN="$package_root/bin/cargo" CARGO_AUDIT_BIN="$package_root/bin/cargo-audit" CARGO_DENY_BIN="$package_root/bin/cargo-deny" BUN_BIN=bun bash "$package_root/scripts/dependency-security.sh" > "$package_root/output.log" 2>&1; then
    printf '%s package section unexpectedly passed\n' "$section" >&2
    exit 1
  fi
  assert_contains "$package_root/output.log" "$section is prohibited"
  ((passed += 1))
done

for package_case in missing_package_manager malformed_package_manager; do
  package_root="$(prepare_fixture "$package_case")"
  if [[ "$package_case" == missing_package_manager ]]; then
    bun -e 'const path = Bun.argv[1]; const value = JSON.parse(await Bun.file(path).text()); delete value.packageManager; await Bun.write(path, JSON.stringify(value));' "$package_root/package.json"
  else
    bun -e 'const path = Bun.argv[1]; const value = JSON.parse(await Bun.file(path).text()); value.packageManager = "bun@latest"; await Bun.write(path, JSON.stringify(value));' "$package_root/package.json"
  fi
  git -C "$package_root" add package.json
  git -C "$package_root" commit -qm "$package_case"
  mkdir -p "$package_root/trace"
  write_fake_tools "$package_root" success
  if RUSTTABLE_SECURITY_TRACE="$package_root/trace" CARGO_BIN="$package_root/bin/cargo" CARGO_AUDIT_BIN="$package_root/bin/cargo-audit" CARGO_DENY_BIN="$package_root/bin/cargo-deny" BUN_BIN=bun bash "$package_root/scripts/dependency-security.sh" > "$package_root/output.log" 2>&1; then
    printf '%s unexpectedly passed\n' "$package_case" >&2
    exit 1
  fi
  assert_contains "$package_root/output.log" 'packageManager must be exactly bun@1.3.14'
  ((passed += 1))
done

lockfile_root="$(prepare_fixture unexpected_bun_lockfile)"
touch "$lockfile_root/bun.lock"
git -C "$lockfile_root" add bun.lock
git -C "$lockfile_root" commit -qm lockfile
mkdir -p "$lockfile_root/trace"
write_fake_tools "$lockfile_root" success
if RUSTTABLE_SECURITY_TRACE="$lockfile_root/trace" CARGO_BIN="$lockfile_root/bin/cargo" CARGO_AUDIT_BIN="$lockfile_root/bin/cargo-audit" CARGO_DENY_BIN="$lockfile_root/bin/cargo-deny" BUN_BIN=bun bash "$lockfile_root/scripts/dependency-security.sh" > "$lockfile_root/output.log" 2>&1; then
  printf 'unexpected Bun lockfile case unexpectedly passed\n' >&2
  exit 1
fi
assert_contains "$lockfile_root/output.log" 'bun.lock is prohibited'
((passed += 1))

printf 'dependency security regression cases: %s passed\n' "$passed"

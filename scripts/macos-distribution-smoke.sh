#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  printf 'macOS distribution smoke requires macOS\n' >&2
  exit 2
fi

root="$(git rev-parse --show-toplevel)"
distribution_directory="$root/target/distribution"
bundle="$root/target/release/bundle/macos/RustTable.app"
archive=""
checksum_file=""
log_file="$distribution_directory/smoke.log"
temporary_log="$distribution_directory/.smoke.log.tmp"
verification_directory="$distribution_directory/verification"

rm -rf "$distribution_directory" "$bundle"
mkdir -p "$distribution_directory"
trap 'rm -f "$temporary_log"' EXIT

pass() {
  printf 'PASS %s\n' "$1" >>"$temporary_log"
}

fail() {
  printf 'FAIL %s\n' "$1" >&2
  exit 1
}

metadata_json="$(cargo metadata --locked --no-deps --format-version 1)"
expected_manifest_path="$(cd "$root/crates/rusttable-app" && pwd)/Cargo.toml"
version="$(printf '%s' "$metadata_json" | EXPECTED_MANIFEST_PATH="$expected_manifest_path" bun -e '
const metadata = JSON.parse(await new Response(Bun.stdin).text());
const expected = process.env.EXPECTED_MANIFEST_PATH;
const packages = metadata.packages.filter((item) => item.name === "rusttable-app" && item.manifest_path === expected);
if (packages.length !== 1 || typeof packages[0].version !== "string") throw new Error("expected one exact rusttable-app package");
process.stdout.write(packages[0].version);
')"
[[ -n "$version" ]] || fail 'cargo package version is empty'

printf 'schema_version=1\n' >"$temporary_log"
printf 'cargo_package_version=%s\n' "$version" >>"$temporary_log"
printf 'git_commit=%s\n' "$(git -C "$root" rev-parse HEAD)" >>"$temporary_log"

bun run install:computer-use -- --compact --no-install --no-launch
pass 'bundle-build'

plist="$bundle/Contents/Info.plist"
[[ -f "$plist" ]] || fail 'staged plist exists'
plutil -lint "$plist" >/dev/null
pass 'staged-plist-lint'

assert_field() {
  local key="$1"
  local expected="$2"
  local actual
  actual="$(plutil -extract "$key" raw -o - "$plist")"
  [[ "$actual" == "$expected" ]] || fail "staged-field-$key"
  pass "staged-field-$key"
}

assert_payload() {
  local candidate="$1"
  local candidate_plist="$candidate/Contents/Info.plist"
  local actual_payload
  local expected_payload
  expected_payload=$'Contents\nContents/Info.plist\nContents/MacOS\nContents/MacOS/RustTable\nContents/Resources\nContents/Resources/LICENSE'
  actual_payload="$(cd "$candidate" && find Contents -print | sort)"
  [[ "$actual_payload" == "$expected_payload" ]] || fail "payload-$candidate"
  [[ -x "$candidate/Contents/MacOS/RustTable" ]] || fail "executable-$candidate"
  [[ -f "$candidate_plist" ]] || fail "plist-$candidate"
  plutil -lint "$candidate_plist" >/dev/null || fail "plist-lint-$candidate"
  cmp "$root/LICENSE" "$candidate/Contents/Resources/LICENSE" >/dev/null || fail "license-$candidate"
  pass "payload-$candidate"
}

assert_field CFBundleDisplayName RustTable
assert_field CFBundleName RustTable
assert_field CFBundleExecutable RustTable
assert_field CFBundleIdentifier com.cgasgarth.rusttable
assert_field CFBundlePackageType APPL
assert_field CFBundleShortVersionString "$version"
assert_field CFBundleVersion "$version"
assert_payload "$bundle"

diagnostics_directory="$distribution_directory/version-diagnostics"
rm -rf "$diagnostics_directory"
set +e
RUSTTABLE_DIAGNOSTICS_DIR="$diagnostics_directory" "$bundle/Contents/MacOS/RustTable" --version >"$distribution_directory/version.stdout" 2>"$distribution_directory/version.stderr"
probe_status=$?
set -e
[[ "$probe_status" -eq 0 ]] || fail 'staged-version-status'
printf 'RustTable %s\n' "$version" | cmp -s - "$distribution_directory/version.stdout" || fail 'staged-version-stdout'
[[ ! -s "$distribution_directory/version.stderr" ]] || fail 'staged-version-stderr'
[[ ! -e "$diagnostics_directory" ]] || fail 'staged-version-diagnostics-side-effect'
pass 'staged-version-probe'

archive="$distribution_directory/RustTable-${version}-macos-unsigned.zip"
checksum_file="$archive.sha256"
COPYFILE_DISABLE=1 ditto -c -k --keepParent "$bundle" "$archive"
[[ -s "$archive" ]] || fail 'archive-created'
pass 'archive-created'
shasum -a 256 "$archive" >"$checksum_file"
shasum -a 256 -c "$checksum_file" >/dev/null
pass 'archive-checksum'

archive_entries="$(unzip -Z1 "$archive" | sort)"
archive_files="$(printf '%s\n' "$archive_entries" | grep -v '/$' || true)"
expected_archive_files=$'RustTable.app/Contents/Info.plist\nRustTable.app/Contents/MacOS/RustTable\nRustTable.app/Contents/Resources/LICENSE'
[[ "$archive_files" == "$expected_archive_files" ]] || fail 'archive-payload'
if printf '%s\n' "$archive_entries" | grep -Eq '(^|/)(__MACOSX/|\._)'; then
  fail 'archive-sidecars'
fi
pass 'archive-payload'

mkdir -p "$verification_directory"
ditto -x -k "$archive" "$verification_directory"
[[ "$(find "$verification_directory" -mindepth 1 -maxdepth 1 -print)" == "$verification_directory/RustTable.app" ]] || fail 'extracted-top-level'
extracted_bundle="$verification_directory/RustTable.app"
plist="$extracted_bundle/Contents/Info.plist"
assert_field() {
  local key="$1"
  local expected="$2"
  local actual
  actual="$(plutil -extract "$key" raw -o - "$plist")"
  [[ "$actual" == "$expected" ]] || fail "extracted-field-$key"
  pass "extracted-field-$key"
}
assert_field CFBundleDisplayName RustTable
assert_field CFBundleName RustTable
assert_field CFBundleExecutable RustTable
assert_field CFBundleIdentifier com.cgasgarth.rusttable
assert_field CFBundlePackageType APPL
assert_field CFBundleShortVersionString "$version"
assert_field CFBundleVersion "$version"
assert_payload "$extracted_bundle"

archive_diagnostics_directory="$distribution_directory/archive-diagnostics"
rm -rf "$archive_diagnostics_directory"
set +e
RUSTTABLE_DIAGNOSTICS_DIR="$archive_diagnostics_directory" "$extracted_bundle/Contents/MacOS/RustTable" --version >"$distribution_directory/archive-version.stdout" 2>"$distribution_directory/archive-version.stderr"
probe_status=$?
set -e
[[ "$probe_status" -eq 0 ]] || fail 'archived-version-status'
printf 'RustTable %s\n' "$version" | cmp -s - "$distribution_directory/archive-version.stdout" || fail 'archived-version-stdout'
[[ ! -s "$distribution_directory/archive-version.stderr" ]] || fail 'archived-version-stderr'
[[ ! -e "$archive_diagnostics_directory" ]] || fail 'archived-version-diagnostics-side-effect'
pass 'archived-version-probe'

archive_size="$(stat -f %z "$archive")"
archive_checksum="$(awk '{print $1}' "$checksum_file")"
printf 'archive_basename=%s\narchive_size=%s\narchive_sha256=%s\n' "$(basename "$archive")" "$archive_size" "$archive_checksum" >>"$temporary_log"
pass 'smoke-complete'
rm -rf "$verification_directory" "$diagnostics_directory" "$archive_diagnostics_directory" "$distribution_directory/version.stdout" "$distribution_directory/version.stderr" "$distribution_directory/archive-version.stdout" "$distribution_directory/archive-version.stderr"
mv "$temporary_log" "$log_file"
trap - EXIT

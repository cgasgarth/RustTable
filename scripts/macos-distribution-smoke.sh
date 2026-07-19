#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  printf 'macOS distribution smoke requires macOS\n' >&2
  exit 2
fi

required_tools=(uname git cargo bun rustc lipo plutil ditto shasum stat unzip cmp find sort awk grep rm mkdir)
for tool in "${required_tools[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'required tool is missing: %s\n' "$tool" >&2
    exit 2
  }
done

root="$(git rev-parse --show-toplevel)"
bun scripts/platform-support.ts --target-os macos --target-architecture aarch64 >/dev/null || {
  printf 'macOS distribution target is absent from platform contract\n' >&2
  exit 1
}
distribution_directory="$root/target/distribution"
bundle="$root/target/release/bundle/macos/RustTable.app"
executable="$bundle/Contents/MacOS/RustTable"
log_file="$distribution_directory/smoke.log"
temporary_log="$distribution_directory/.smoke.log.tmp"
verification_directory="$distribution_directory/verification"
work_directory="$distribution_directory/.work"
pass_records_file="$work_directory/pass-records"

cleanup() {
  local status="$?"
  rm -f "$temporary_log"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  printf 'FAIL %s\n' "$1" >&2
  exit 1
}

pass() {
  printf '%s\n' "$1" >>"$pass_records_file"
}

capture() {
  local output="$1"
  shift
  set +e
  "$@" >"$output" 2>&1
  capture_status=$?
  set -e
}

sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

file_size() {
  stat -f '%z' "$1"
}

file_mode() {
  stat -f '%p' "$1"
}

rm -rf "$distribution_directory" "$bundle"
mkdir -p "$distribution_directory"
work_directory="$(mktemp -d "$distribution_directory/.work.XXXXXX")"
pass_records_file="$work_directory/pass-records"

metadata_json="$work_directory/cargo-metadata.json"
cargo metadata --locked --no-deps --format-version 1 >"$metadata_json"
expected_manifest_path="$(cd "$root/crates/rusttable-app" && pwd -P)/Cargo.toml"
version="$(EXPECTED_MANIFEST_PATH="$expected_manifest_path" bun -e '
const metadata = JSON.parse(await new Response(Bun.stdin).text());
const expected = process.env.EXPECTED_MANIFEST_PATH;
const packages = metadata.packages.filter((item) => item.name === "rusttable-app" && item.manifest_path === expected);
if (packages.length !== 1 || typeof packages[0].version !== "string") throw new Error("expected one exact rusttable-app package");
process.stdout.write(packages[0].version);
' <"$metadata_json")"
[[ -n "$version" ]] || fail 'cargo-package-version-empty'

git_sha="$(git -C "$root" rev-parse HEAD)"
rustc_output="$work_directory/rustc-vV"
lipo_output="$work_directory/staged-lipo-archs"
identity_json="$work_directory/staged-identity.json"

bun run install:computer-use -- --compact --no-install --no-launch
pass 'bundle-build'

plist="$bundle/Contents/Info.plist"
[[ -f "$plist" ]] || fail 'staged-plist-exists'
plutil -lint "$plist" >/dev/null || fail 'staged-plist-lint'
pass 'staged-plist-lint'

assert_field() {
  local label="$1"
  local key="$2"
  local expected="$3"
  local actual
  actual="$(plutil -extract "$key" raw -o - "$plist")"
  [[ "$actual" == "$expected" ]] || fail "${label}-field-${key}"
  pass "${label}-field-${key}"
}

assert_payload() {
  local label="$1"
  local candidate="$2"
  local candidate_plist="$candidate/Contents/Info.plist"
  local actual_payload
  local expected_payload
  expected_payload=$'Contents\nContents/Info.plist\nContents/MacOS\nContents/MacOS/RustTable\nContents/Resources\nContents/Resources/LICENSE'
  actual_payload="$(cd "$candidate" && find Contents -print | sort)"
  [[ "$actual_payload" == "$expected_payload" ]] || fail "${label}-payload"
  [[ -x "$candidate/Contents/MacOS/RustTable" ]] || fail "${label}-executable"
  [[ -f "$candidate_plist" ]] || fail "${label}-plist"
  plutil -lint "$candidate_plist" >/dev/null || fail "${label}-plist-lint"
  cmp "$root/LICENSE" "$candidate/Contents/Resources/LICENSE" >/dev/null || fail "${label}-license"
  pass "${label}-payload"
}

assert_field staged CFBundleDisplayName RustTable
assert_field staged CFBundleName RustTable
assert_field staged CFBundleExecutable RustTable
assert_field staged CFBundleIdentifier com.cgasgarth.rusttable
assert_field staged CFBundlePackageType APPL
assert_field staged CFBundleShortVersionString "$version"
assert_field staged CFBundleVersion "$version"
assert_payload staged "$bundle"

capture "$rustc_output" rustc -vV
[[ "$capture_status" -eq 0 ]] || fail 'staged-rustc-version'
capture "$lipo_output" lipo -archs "$executable"
[[ "$capture_status" -eq 0 ]] || fail 'staged-lipo-architecture'
bun scripts/macos-artifact-identity.ts "$rustc_output" "$lipo_output" "$version" >"$identity_json" || fail 'staged-artifact-identity'
pass 'staged-rustc-identity'
pass 'staged-target-architecture'
pass 'staged-mach-o-architecture'

staged_executable_sha256="$(sha256 "$executable")"
staged_executable_size="$(file_size "$executable")"
staged_executable_mode="$(file_mode "$executable")"
[[ -x "$executable" ]] || fail 'staged-executable-mode'
pass 'staged-executable-sha256'
pass 'staged-executable-size'
pass 'staged-executable-mode'

diagnostics_directory="$distribution_directory/version-diagnostics"
rm -rf "$diagnostics_directory"
set +e
RUSTTABLE_DIAGNOSTICS_DIR="$diagnostics_directory" "$executable" --version >"$work_directory/version.stdout" 2>"$work_directory/version.stderr"
probe_status=$?
set -e
[[ "$probe_status" -eq 0 ]] || fail 'staged-version-status'
pass 'staged-version-status'
printf 'RustTable %s\n' "$version" | cmp -s - "$work_directory/version.stdout" || fail 'staged-version-stdout'
pass 'staged-version-stdout'
[[ ! -s "$work_directory/version.stderr" ]] || fail 'staged-version-stderr'
pass 'staged-version-stderr'
[[ ! -e "$diagnostics_directory" ]] || fail 'staged-version-diagnostics-side-effect'
pass 'staged-version-diagnostics-side-effect'
pass 'staged-version-probe'

archive_basename="$(bun -e 'process.stdout.write(JSON.parse(await new Response(Bun.stdin).text()).archiveBasename)' <"$identity_json")"
checksum_basename="${archive_basename}.sha256"
archive="$distribution_directory/$archive_basename"
checksum_file="$distribution_directory/$checksum_basename"
[[ "$archive_basename" == RustTable-*-*'-unsigned.zip' ]] || fail 'target-qualified-archive-basename'
[[ "$archive_basename" != RustTable-*-macos-unsigned.zip ]] || fail 'architecture-ambiguous-archive-basename'
pass 'target-qualified-archive-basename'

COPYFILE_DISABLE=1 ditto -c -k --keepParent "$bundle" "$archive"
[[ -s "$archive" ]] || fail 'archive-created'
pass 'archive-created'
(cd "$distribution_directory" && shasum -a 256 "$archive_basename" >"$checksum_basename")
(cd "$distribution_directory" && shasum -a 256 -c "$checksum_basename" >/dev/null)
archive_sha256="$(awk '{print $1}' "$checksum_file")"
archive_size="$(file_size "$archive")"
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
pass 'extracted-top-level'
extracted_bundle="$verification_directory/RustTable.app"
plist="$extracted_bundle/Contents/Info.plist"
assert_field extracted CFBundleDisplayName RustTable
assert_field extracted CFBundleName RustTable
assert_field extracted CFBundleExecutable RustTable
assert_field extracted CFBundleIdentifier com.cgasgarth.rusttable
assert_field extracted CFBundlePackageType APPL
assert_field extracted CFBundleShortVersionString "$version"
assert_field extracted CFBundleVersion "$version"
assert_payload extracted "$extracted_bundle"

extracted_lipo_output="$work_directory/extracted-lipo-archs"
extracted_identity_json="$work_directory/extracted-identity.json"
extracted_executable="$extracted_bundle/Contents/MacOS/RustTable"
capture "$extracted_lipo_output" lipo -archs "$extracted_executable"
[[ "$capture_status" -eq 0 ]] || fail 'archived-lipo-architecture'
bun scripts/macos-artifact-identity.ts "$rustc_output" "$extracted_lipo_output" "$version" >"$extracted_identity_json" || fail 'archived-artifact-identity'
cmp -s "$identity_json" "$extracted_identity_json" || fail 'archived-artifact-identity-match'
pass 'archived-target-architecture'
pass 'archived-mach-o-architecture'
pass 'archived-artifact-identity'

cmp -s "$executable" "$extracted_executable" || fail 'archived-executable-bytes'
pass 'archived-executable-bytes'
extracted_executable_sha256="$(sha256 "$extracted_executable")"
extracted_executable_size="$(file_size "$extracted_executable")"
extracted_executable_mode="$(file_mode "$extracted_executable")"
[[ "$extracted_executable_sha256" == "$staged_executable_sha256" ]] || fail 'archived-executable-sha256'
pass 'archived-executable-sha256'
[[ "$extracted_executable_size" == "$staged_executable_size" ]] || fail 'archived-executable-size'
pass 'archived-executable-size'
[[ "$extracted_executable_mode" == "$staged_executable_mode" ]] || fail 'archived-executable-mode'
pass 'archived-executable-mode'
pass 'archived-executable-byte-identity'

archive_diagnostics_directory="$distribution_directory/archive-diagnostics"
rm -rf "$archive_diagnostics_directory"
set +e
RUSTTABLE_DIAGNOSTICS_DIR="$archive_diagnostics_directory" "$extracted_executable" --version >"$work_directory/archive-version.stdout" 2>"$work_directory/archive-version.stderr"
probe_status=$?
set -e
[[ "$probe_status" -eq 0 ]] || fail 'archived-version-status'
pass 'archived-version-status'
printf 'RustTable %s\n' "$version" | cmp -s - "$work_directory/archive-version.stdout" || fail 'archived-version-stdout'
pass 'archived-version-stdout'
[[ ! -s "$work_directory/archive-version.stderr" ]] || fail 'archived-version-stderr'
pass 'archived-version-stderr'
[[ ! -e "$archive_diagnostics_directory" ]] || fail 'archived-version-diagnostics-side-effect'
pass 'archived-version-diagnostics-side-effect'
pass 'archived-version-probe'
(cd "$distribution_directory" && shasum -a 256 -c "$checksum_basename" >/dev/null)
pass 'final-checksum-verification'

pass 'smoke-complete'
bun scripts/macos-artifact-identity.ts --render-log \
  "$identity_json" "$git_sha" com.cgasgarth.rusttable "$archive_sha256" "$archive_size" \
  "$staged_executable_sha256" "$staged_executable_size" "$pass_records_file" >"$temporary_log"
rm -rf "$verification_directory" "$work_directory" "$diagnostics_directory" "$archive_diagnostics_directory"
mv "$temporary_log" "$log_file"
trap - EXIT

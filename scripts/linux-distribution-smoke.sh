#!/usr/bin/env bash
set -euo pipefail
# Ignore inherited job caps so Cargo selects host-detected parallelism.
unset CARGO_BUILD_JOBS

if [[ "$(uname -s)" != Linux ]]; then
  printf 'Linux distribution smoke requires Linux\n' >&2
  exit 2
fi

required_tools=(uname git cargo bun rustc readelf ldd tar gzip sha256sum cmp find sort awk grep sed stat mktemp env cat cp chmod mkdir rm mv)
for tool in "${required_tools[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'required tool is missing: %s\n' "$tool" >&2
    exit 2
  }
done
tar --version 2>/dev/null | grep -q '^tar (GNU tar)' || {
  printf 'GNU tar is required for deterministic Linux distribution archives\n' >&2
  exit 2
}
export LC_ALL=C

root="$(git rev-parse --show-toplevel)"
distribution_directory="$root/target/linux-distribution"
stage_directory="$distribution_directory/stage"
work_directory="$distribution_directory/.work"
pass_log="$work_directory/pass-records"
log_file="$distribution_directory/smoke.log"
temporary_log="$distribution_directory/.smoke.log.tmp"
verification_directory="$distribution_directory/verification"

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
  printf 'pass=%s\n' "$1" >>"$pass_log"
}

capture() {
  local output="$1"
  shift
  set +e
  "$@" >"$output" 2>&1
  capture_status=$?
  set -e
}

rm -rf "$distribution_directory"
mkdir -p "$work_directory" "$stage_directory"
: >"$pass_log"

metadata_json="$work_directory/cargo-metadata.json"
cargo metadata --locked --no-deps --format-version 1 >"$metadata_json"
expected_manifest_path="$(cd "$root/crates/rusttable-app" && pwd -P)/Cargo.toml"
version="$(EXPECTED_MANIFEST_PATH="$expected_manifest_path" bun -e '
const metadata = JSON.parse(await new Response(Bun.stdin).text());
const expected = process.env.EXPECTED_MANIFEST_PATH;
const packages = metadata.packages.filter((item) => item.name === "rusttable-app" && item.manifest_path === expected);
if (packages.length !== 1 || typeof packages[0].version !== "string") throw new Error("expected one exact rusttable-app package");
process.stdout.write(packages[0].version);
' <"$metadata_json")" || fail 'cargo-package-version'
[[ -n "$version" ]] || fail 'cargo-package-version-empty'
target_triple="$(bun scripts/platform-support.ts --target-os linux --target-architecture x86_64 | awk 'NF { print; exit }')" || fail 'platform-contract'
[[ -n "$target_triple" ]] || fail 'platform-target-empty'
archive_basename="RustTable-${version}-${target_triple}-unsigned.tar.gz"
git_sha="$(git -C "$root" rev-parse HEAD)"

cargo build --release --package rusttable-app --bin rusttable-app --locked
release_binary="$root/target/release/rusttable-app"
[[ -f "$release_binary" ]] || fail 'release-binary-exists'

stage_top="$stage_directory/RustTable-${version}-${target_triple}"
mkdir -p "$stage_top/bin"
cp -- "$release_binary" "$stage_top/bin/RustTable"
cp -- "$root/LICENSE" "$stage_top/LICENSE"
chmod 755 "$stage_top/bin/RustTable"
chmod 644 "$stage_top/LICENSE"

stage_payload="$work_directory/stage-payload"
(cd "$stage_top" && find . -mindepth 1 -maxdepth 2 -print | sed 's#^./##' | sort) >"$stage_payload"
expected_payload=$'LICENSE\nbin\nbin/RustTable'
cmp -s <(printf '%s\n' "$expected_payload") "$stage_payload" || fail 'staged-payload'
[[ -f "$stage_top/bin/RustTable" && -x "$stage_top/bin/RustTable" ]] || fail 'staged-executable'
[[ "$(stat -c '%a' "$stage_top/bin/RustTable")" == 755 ]] || fail 'staged-executable-mode'
[[ "$(stat -c '%a' "$stage_top/LICENSE")" == 644 ]] || fail 'staged-license-mode'
cmp -s "$root/LICENSE" "$stage_top/LICENSE" || fail 'staged-license'
pass 'staged-payload'

rustc_output="$work_directory/rustc-vV"
elf_header_output="$work_directory/readelf-h"
elf_program_output="$work_directory/readelf-l"
elf_dynamic_output="$work_directory/readelf-d"
ldd_output="$work_directory/ldd"
capture "$rustc_output" rustc -vV
[[ "$capture_status" -eq 0 ]] || fail 'rustc-version'
capture "$elf_header_output" env LC_ALL=C readelf -h "$stage_top/bin/RustTable"
[[ "$capture_status" -eq 0 ]] || fail 'staged-elf-header'
capture "$elf_program_output" env LC_ALL=C readelf -l "$stage_top/bin/RustTable"
[[ "$capture_status" -eq 0 ]] || fail 'staged-elf-program-headers'
capture "$elf_dynamic_output" env LC_ALL=C readelf -d "$stage_top/bin/RustTable"
[[ "$capture_status" -eq 0 ]] || fail 'staged-elf-dynamic'
capture "$ldd_output" env LC_ALL=C ldd "$stage_top/bin/RustTable"
ldd_status="$capture_status"

identity_json="$work_directory/staged-identity.json"
bun scripts/linux-artifact-identity.ts \
  "$rustc_output" "$elf_header_output" "$elf_program_output" "$elf_dynamic_output" "$ldd_output" \
  "$ldd_status" "$version" "$archive_basename" >"$identity_json" || fail 'staged-artifact-identity'

identity_records="$work_directory/identity-records"
cat "$identity_json" | bun -e '
const value = JSON.parse(await new Response(Bun.stdin).text());
const records = [
  `rust_release=${value.rustRelease}`,
  `rust_host=${value.rustHost}`,
  `elf_class=${value.elfClass}`,
  `elf_data=${value.elfData}`,
  `elf_machine=${value.elfMachine}`,
  `elf_type=${value.elfType}`,
  `program_interpreter=${value.interpreter}`,
  ...value.needed.map((name) => `needed_library=${name}`),
];
process.stdout.write(`${records.join("\n")}\n`);
' >"$identity_records"

pass 'staged-identity'

run_version_probe() {
  local label="$1"
  local executable="$2"
  local diagnostics="$3"
  local stdout_file="$4"
  local stderr_file="$5"
  rm -rf "$diagnostics"
  set +e
  RUSTTABLE_DIAGNOSTICS_DIR="$diagnostics" "$executable" --version >"$stdout_file" 2>"$stderr_file"
  local status=$?
  set -e
  [[ "$status" -eq 0 ]] || fail "${label}-version-status"
  printf 'RustTable %s\n' "$version" | cmp -s - "$stdout_file" || fail "${label}-version-stdout"
  [[ ! -s "$stderr_file" ]] || fail "${label}-version-stderr"
  [[ ! -e "$diagnostics" ]] || fail "${label}-version-diagnostics-side-effect"
  rm -f "$stdout_file" "$stderr_file"
  pass "${label}-version-probe"
}

run_version_probe staged "$stage_top/bin/RustTable" "$work_directory/staged-diagnostics" "$work_directory/staged-version.stdout" "$work_directory/staged-version.stderr"

archive_one="$work_directory/archive-one.tar.gz"
archive_two="$work_directory/archive-two.tar.gz"
make_archive() {
  local destination="$1"
  tar --sort=name --format=ustar --owner=0 --group=0 --numeric-owner --mode='u+rwX,go+rX,go-w' --mtime='@0' -C "$stage_directory" -cf - "$(basename "$stage_top")" | gzip -n >"$destination"
}
make_archive "$archive_one"
make_archive "$archive_two"
cmp -s "$archive_one" "$archive_two" || fail 'archive-determinism'
archive="$distribution_directory/$archive_basename"
cp -- "$archive_one" "$archive"
[[ -s "$archive" ]] || fail 'archive-created'
archive_sha256="$(sha256sum "$archive" | awk '{print $1}')"
archive_size="$(stat -c '%s' "$archive")"
pass 'archive-determinism'

checksum_file="$archive.sha256"
checksum_temporary="$distribution_directory/.$archive_basename.sha256.tmp"
printf '%s  %s\n' "$archive_sha256" "$archive_basename" >"$checksum_temporary"
grep -Eq '^[0-9a-f]{64}  [^/[:space:]]+\.tar\.gz$' "$checksum_temporary" || fail 'checksum-record'
[[ "$(cat "$checksum_temporary")" == "$archive_sha256  $archive_basename" ]] || fail 'checksum-basename'
(
  cd "$distribution_directory"
  sha256sum -c "$(basename "$checksum_temporary")" >/dev/null
) || fail 'checksum-verification'
mv -- "$checksum_temporary" "$checksum_file"
(
  cd "$distribution_directory"
  sha256sum -c "$(basename "$checksum_file")" >/dev/null
) || fail 'checksum-promotion'
pass 'archive-checksum'

archive_entries="$work_directory/archive-entries"
tar -tzf "$archive" | sort >"$archive_entries"
expected_entries=$(printf '%s\n' \
  "$(basename "$stage_top")/" \
  "$(basename "$stage_top")/LICENSE" \
  "$(basename "$stage_top")/bin/" \
  "$(basename "$stage_top")/bin/RustTable" | sort)
cmp -s <(printf '%s\n' "$expected_entries") "$archive_entries" || fail 'archive-payload'
while IFS= read -r entry; do
  [[ "$entry" != /* && "$entry" != ../* && "$entry" != */../* && "$entry" != *'/..' ]] || fail 'archive-unsafe-path'
done <"$archive_entries"
pass 'archive-payload'

rm -rf "$verification_directory"
mkdir -p "$verification_directory"
tar --extract --file "$archive" --directory "$verification_directory" --no-same-owner || fail 'archive-extraction'
extracted_top="$verification_directory/$(basename "$stage_top")"
[[ -d "$extracted_top" ]] || fail 'extracted-top-level'
extracted_payload="$work_directory/extracted-payload"
(cd "$extracted_top" && find -P . -mindepth 1 -maxdepth 2 -print | sed 's#^./##' | sort) >"$extracted_payload"
cmp -s "$stage_payload" "$extracted_payload" || fail 'extracted-payload'
[[ -z "$(find -P "$extracted_top" \( -type l -o -type b -o -type c -o -type p -o -type s -o \( -type f -links +1 \) \) -print)" ]] || fail 'extracted-special-file'
[[ -z "$(find -P "$extracted_top" \( -perm -0002 -o -perm -4000 -o -perm -2000 \) -print)" ]] || fail 'extracted-unsafe-mode'
cmp -s "$stage_top/bin/RustTable" "$extracted_top/bin/RustTable" || fail 'extracted-executable-bytes'
cmp -s "$root/LICENSE" "$extracted_top/LICENSE" || fail 'extracted-license-bytes'
[[ "$(sha256sum "$stage_top/bin/RustTable" | awk '{print $1}')" == "$(sha256sum "$extracted_top/bin/RustTable" | awk '{print $1}')" ]] || fail 'extracted-executable-sha256'
[[ "$(stat -c '%s' "$stage_top/bin/RustTable")" == "$(stat -c '%s' "$extracted_top/bin/RustTable")" ]] || fail 'extracted-executable-size'
[[ "$(stat -c '%a' "$stage_top/bin/RustTable")" == "$(stat -c '%a' "$extracted_top/bin/RustTable")" ]] || fail 'extracted-executable-mode'
[[ "$(stat -c '%a' "$extracted_top/LICENSE")" == 644 ]] || fail 'extracted-license-mode'
pass 'extracted-payload'

extracted_rustc="$work_directory/extracted-rustc-vV"
extracted_elf_header="$work_directory/extracted-readelf-h"
extracted_elf_program="$work_directory/extracted-readelf-l"
extracted_elf_dynamic="$work_directory/extracted-readelf-d"
extracted_ldd="$work_directory/extracted-ldd"
capture "$extracted_rustc" rustc -vV
capture "$extracted_elf_header" env LC_ALL=C readelf -h "$extracted_top/bin/RustTable"
capture "$extracted_elf_program" env LC_ALL=C readelf -l "$extracted_top/bin/RustTable"
capture "$extracted_elf_dynamic" env LC_ALL=C readelf -d "$extracted_top/bin/RustTable"
capture "$extracted_ldd" env LC_ALL=C ldd "$extracted_top/bin/RustTable"
extracted_ldd_status="$capture_status"
extracted_identity="$work_directory/extracted-identity.json"
bun scripts/linux-artifact-identity.ts \
  "$extracted_rustc" "$extracted_elf_header" "$extracted_elf_program" "$extracted_elf_dynamic" "$extracted_ldd" \
  "$extracted_ldd_status" "$version" "$archive_basename" >"$extracted_identity" || fail 'extracted-artifact-identity'
cmp -s "$identity_json" "$extracted_identity" || fail 'extracted-artifact-identity-match'
pass 'extracted-identity'
run_version_probe extracted "$extracted_top/bin/RustTable" "$work_directory/extracted-diagnostics" "$work_directory/extracted-version.stdout" "$work_directory/extracted-version.stderr"
(
  cd "$distribution_directory"
  sha256sum -c "$(basename "$checksum_file")" >/dev/null
) || fail 'final-checksum-verification'

executable_sha256="$(sha256sum "$stage_top/bin/RustTable" | awk '{print $1}')"
executable_size="$(stat -c '%s' "$stage_top/bin/RustTable")"
{
  printf 'schema=%s\n' 'RUSTTABLE_LINUX_DISTRIBUTION_V1'
  printf 'git_sha=%s\n' "$git_sha"
  printf 'cargo_package_version=%s\n' "$version"
  cat "$identity_records"
  printf 'executable_sha256=%s\n' "$executable_sha256"
  printf 'executable_size=%s\n' "$executable_size"
  printf 'archive_basename=%s\n' "$archive_basename"
  printf 'checksum_basename=%s\n' "$(basename "$checksum_file")"
  printf 'archive_sha256=%s\n' "$archive_sha256"
  printf 'archive_size=%s\n' "$archive_size"
  cat "$pass_log"
  printf 'pass=%s\n' 'smoke-complete'
} >"$temporary_log"
rm -rf "$verification_directory" "$work_directory"
mv -- "$temporary_log" "$log_file"
trap - EXIT

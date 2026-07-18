#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "$script_directory/.." && pwd -P)"
git_root="$(git -C "$repository_root" rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$git_root" ]]; then
  printf 'dependency security: repository root is not a Git worktree\n' >&2
  exit 1
fi
repository_root="$(cd -- "$git_root" && pwd)"
security_directory="$repository_root/target/security"
versions_file="$script_directory/security-tool-versions.env"

tracked_file() {
  local file="$1"
  git -C "$repository_root" ls-files --error-unmatch -- "$file" >/dev/null 2>&1
}

for required_file in Cargo.lock Cargo.toml deny.toml package.json; do
  if [[ ! -f "$repository_root/$required_file" ]] || ! tracked_file "$required_file"; then
    printf 'dependency security: required tracked file is missing: %s\n' "$required_file" >&2
    exit 1
  fi
done
if [[ ! -f "$versions_file" ]] || ! tracked_file scripts/security-tool-versions.env; then
  printf 'dependency security: exact tool version file is missing or untracked\n' >&2
  exit 1
fi

read_version() {
  local key="$1"
  local value
  value="$(awk -F= -v key="$key" '$1 == key { print $2; count++ } END { if (count != 1) exit 1 }' "$versions_file")" || {
    printf 'dependency security: malformed version key: %s\n' "$key" >&2
    exit 1
  }
  if [[ ! "$value" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf 'dependency security: malformed version for %s: %s\n' "$key" "$value" >&2
    exit 1
  fi
  printf '%s' "$value"
}

audit_version="$(read_version CARGO_AUDIT_VERSION)"
deny_version="$(read_version CARGO_DENY_VERSION)"
cargo_bin="${CARGO_BIN:-cargo}"
audit_bin="${CARGO_AUDIT_BIN:-cargo-audit}"
deny_bin="${CARGO_DENY_BIN:-cargo-deny}"
bun_bin="${BUN_BIN:-bun}"

rm -rf "$security_directory"
mkdir -p "$security_directory"

fail_stage() {
  local stage="$1"
  shift
  printf 'dependency security: %s failed\n' "$stage" >&2
  if (( $# > 0 )); then
    cat "$@" >&2 || true
  fi
  printf 'final_status=fail\nfailed_stage=%s\n' "$stage" > "$security_directory/summary.txt"
  exit 1
}

if ! audit_version_output="$($audit_bin --version 2>&1)" || [[ "$audit_version_output" != *"cargo-audit $audit_version"* ]]; then
  printf '%s\n' "$audit_version_output" > "$security_directory/audit-version.log"
  fail_stage audit-version "$security_directory/audit-version.log"
fi
if ! deny_version_output="$($deny_bin --version 2>&1)" || [[ "$deny_version_output" != *"cargo-deny $deny_version"* ]]; then
  printf '%s\n' "$deny_version_output" > "$security_directory/deny-version.log"
  fail_stage deny-version "$security_directory/deny-version.log"
fi

if ! bun_version_output="$($bun_bin --version 2>&1)" || [[ "$bun_version_output" != "1.3.14" ]]; then
  printf '%s\n' "$bun_version_output" > "$security_directory/bun-version.log"
  fail_stage bun-version "$security_directory/bun-version.log"
fi
if ! RUSTTABLE_PACKAGE_MANIFEST="$repository_root/package.json" "$bun_bin" "$script_directory/validate-package-manifest.ts" > "$security_directory/javascript-manifest.log" 2>&1; then
  fail_stage javascript-manifest "$security_directory/javascript-manifest.log"
fi

if ! "$cargo_bin" metadata --locked --format-version 1 > "$security_directory/metadata.json" 2> "$security_directory/metadata.log"; then
  fail_stage metadata "$security_directory/metadata.log"
fi
if ! "$audit_bin" audit --file "$repository_root/Cargo.lock" --json > "$security_directory/audit.json" 2> "$security_directory/audit.log"; then
  fail_stage audit "$security_directory/audit.log"
fi
if ! "$deny_bin" check bans licenses sources > "$security_directory/deny.log" 2>&1; then
  fail_stage deny "$security_directory/deny.log"
fi
if ! "$cargo_bin" tree --workspace --all-features --duplicates --locked > "$security_directory/duplicates.txt" 2> "$security_directory/duplicates.log"; then
  fail_stage duplicates "$security_directory/duplicates.log"
fi

lock_hash_command=(shasum -a 256)
if ! command -v shasum >/dev/null 2>&1; then
  lock_hash_command=(sha256sum)
fi
lock_hash="$("${lock_hash_command[@]}" "$repository_root/Cargo.lock" | awk '{print $1}')"
workspace_packages="$(awk '/^members = \[/,/^\]/ { if ($0 ~ /"/) count++ } END { print count + 0 }' "$repository_root/Cargo.toml")"
duplicate_status="reported"
javascript_packages=0
{
  printf 'rust_version=%s\n' "$(rustc --version)"
  printf 'bun_version=%s\n' "$bun_version_output"
  printf 'cargo_audit_version=%s\n' "$audit_version"
  printf 'cargo_deny_version=%s\n' "$deny_version"
  printf 'cargo_lock_sha256=%s\n' "$lock_hash"
  printf 'workspace_packages=%s\n' "$workspace_packages"
  printf 'duplicate_inventory=%s\n' "$duplicate_status"
  printf 'exception_ids=none\n'
  printf 'javascript_dependency_count=%s\n' "$javascript_packages"
  printf 'final_status=pass\n'
} > "$security_directory/summary.txt"
printf 'dependency security: pass (%s)\n' "$security_directory/summary.txt"

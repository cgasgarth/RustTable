#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd -- "$(dirname -- "$BASH_SOURCE")/.." && pwd)"
toolchain_file="$root_directory/rust-toolchain.toml"
baseline_file="$root_directory/quality/compiler-baseline.toml"

read_field() {
  local file="$1"
  local field="$2"
  sed -nE 's/^'"$field"'[[:space:]]*=[[:space:]]*"([^"]+)"[[:space:]]*$/\1/p' "$file" | head -n 1
}

argument="$1"
case "$argument" in
  channel)
    read_field "$toolchain_file" channel
    ;;
  release)
    read_field "$baseline_file" release
    ;;
  rust-version)
    read_field "$baseline_file" rust_version
    ;;
  components)
    sed -nE 's/^components = \[(.*)\]$/\1/p' "$toolchain_file" | tr -d '"' | tr ',' '\n' | sed '/^[[:space:]]*$/d;s/^[[:space:]]*//;s/[[:space:]]*$//'
    ;;
  *)
    printf 'usage: %s {channel|release|rust-version|components}\n' "$0" >&2
    exit 64
    ;;
esac

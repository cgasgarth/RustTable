#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
readme="${1:-$root/README.md}"

if [[ ! -f "$readme" ]]; then
  printf 'README contract: missing README\n' >&2
  exit 1
fi

if [[ -z "${RUSTTABLE_README_FIXTURE:-}" ]]; then
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' EXIT
  for case_name in badge release master cmake gtk installer workflow; do
    case "$case_name" in
      badge) line='https://img.shields.io/github/actions/workflow/status/darktable-org/darktable/ci.yml' ;;
      release) line='https://github.com/darktable-org/darktable/releases/latest' ;;
      master) line='build the master branch' ;;
      cmake) line='cmake -S . -B build' ;;
      gtk) line='install GTK before building' ;;
      installer) line='curl https://example.invalid/install.sh | sh' ;;
      workflow) line='see .github/workflows/ci.yml' ;;
    esac
    cp "$readme" "$fixture/README.md"
    printf '\n%s\n' "$line" >>"$fixture/README.md"
    if RUSTTABLE_README_FIXTURE=1 bash "$0" "$fixture/README.md" >/dev/null 2>&1; then
      printf 'README contract: fixture accepted (%s)\n' "$case_name" >&2
      exit 1
    fi
  done
fi

if grep -Eiq 'shields\.io/github/actions/workflow/status/darktable-org|darktable-org/darktable/(actions|releases)|(^|[^[:alnum:]_])master([^[:alnum:]_]|$)|(^|[[:space:]])(cmake|CMake|GTK|gtk)([[:space:]]|$)|curl[[:space:]].*\|[[:space:]]*(sh|bash)|powershell.*(irm|iex)|\.github/workflows/(ci|nightly)\.yml' "$readme"; then
  printf 'README contract: prohibited inherited instructions found\n' >&2
  exit 1
fi

printf 'README contract: PASS\n'

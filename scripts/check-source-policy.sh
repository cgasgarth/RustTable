#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
violations=()

violation() {
  violations+=("$1")
}

while IFS= read -r -d '' path; do
  case "$path" in
    data/shortcutsrc|fixtures/*|target/*)
      ;;
    data/*|doc/*|dev-doc/*|po/*|.github/ISSUE_TEMPLATE/*|AUTHORS|.mailmap)
      violation "inherited product material is not allowed: $path"
      ;;
    *.c|*.cc|*.cl|*.cmake|*.cpp|*.cxx|*.h|*.hh|*.hpp|*.m|*.mm|*.s|*.S|CMakeLists.txt|*/CMakeLists.txt)
      violation "native source/build file is not allowed: $path"
      ;;
  esac
done < <(git -C "$root" ls-files --cached --others --exclude-standard -z)

if (( ${#violations[@]} > 0 )); then
  printf 'source-policy: FAIL\n' >&2
  printf ' - %s\n' "${violations[@]}" >&2
  exit 1
fi

printf 'source-policy: PASS\n'

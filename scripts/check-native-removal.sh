#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
violations=()

violation() {
  violations+=("$1")
}

forbidden_paths=(
  src
  cmake
  tools
  packaging
  .ci
  CMakeLists.txt
  ConfigureChecks.cmake
  DefineOptions.cmake
  build.sh
  .gitmodules
  iwyu.imp
  .github/workflows/ci.yml
  .github/workflows/nightly.yml
  .github/workflows/check-po.yml
  data/kernels
  data/CMakeLists.txt
  data/supported_extensions.cmake
  data/pixmaps/CMakeLists.txt
  data/styles/CMakeLists.txt
  doc/CMakeLists.txt
  doc/man/CMakeLists.txt
  doc/man/po/CMakeLists.txt
  po/CMakeLists.txt
)

for path in "${forbidden_paths[@]}"; do
  if [[ -n "$(git -C "$root" ls-files -- "$path")" ]]; then
    violation "forbidden legacy path exists: $path"
  fi
done

while IFS= read -r path; do
  case "$path" in
    *.c|*.h|*.cc|*.cpp|*.cxx|*.hpp|*.m|*.mm|*.asm|*.s|*.S|*.cl|*.cmake|CMakeLists.txt|*/CMakeLists.txt)
      violation "forbidden native source/build file is tracked: $path"
      ;;
  esac
done < <(git -C "$root" ls-files)

entry_points=(
  .github/workflows/rust-pr.yml
  .github/workflows/rust-main.yml
  scripts/main-ci.sh
  scripts/pr-ci.sh
  scripts/precommit-fast.sh
  scripts/prepush-fast.sh
  .githooks/pre-commit
  .githooks/pre-push
)
for path in "${entry_points[@]}"; do
  [[ -f "$root/$path" ]] || continue
  if rg -n '(^|[^[:alnum:]_])(cmake|CMakeLists\.txt|build\.sh|gcc|g\+\+|clang(\+\+)?|ccache|meson|ninja|bindgen|rustc-link-(lib|search)|extern "C"|target_link)' "$root/$path" >/dev/null; then
    violation "native build reference in active entry point: $path"
  fi
done

if (( ${#violations[@]} > 0 )); then
  printf 'native-removal: FAIL\n' >&2
  printf ' - %s\n' "${violations[@]}" >&2
  exit 1
fi

printf 'native-removal: PASS\n'

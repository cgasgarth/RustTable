#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
script="$root/scripts/dev/capture-ui-screenshots.sh"
help="$("$script" --help)"
[[ "$help" == *"default: 1280"* ]]
[[ "$help" == *"default: 768"* ]]
[[ "$help" == *"reference-app"* ]]
[[ "$help" == *"reference-dir"* ]]
[[ "$help" == *"refresh-reference"* ]]
[[ "$help" == *"rusttable-lighttable.png"* ]]
[[ "$help" == *"rusttable-darkroom.png"* ]]
[[ "$help" == *"darktable-lighttable.png"* ]]
[[ "$help" == *"darktable-darkroom.png"* ]]
[[ "$help" == *"manifest.json"* ]]

if "$script" --unknown >/dev/null 2>&1; then
  printf 'unknown capture option unexpectedly succeeded\n' >&2
  exit 1
fi
if "$script" --width 0 >/dev/null 2>&1; then
  printf 'invalid capture width unexpectedly succeeded\n' >&2
  exit 1
fi
if "$script" --run-id '../escape' >/dev/null 2>&1; then
  printf 'unsafe capture run ID unexpectedly succeeded\n' >&2
  exit 1
fi

printf 'UI screenshot capture argument contract passed\n'

#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
script="$root/scripts/dev/capture-ui-screenshots.sh"
help="$("$script" --help)"
[[ "$help" == *"--allow-foreground"* ]]
[[ "$help" == *"default: current screen usable width"* ]]
[[ "$help" == *"default: current screen usable height"* ]]
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

foreground_error="$("$script" --run-id contract 2>&1 || true)"
[[ "$foreground_error" == *"rerun with --allow-foreground"* ]]
grep -Fq "screen.visibleFrame" "$script"
grep -Fq 'set value of attribute "AXFullScreen" of window 1 to false' "$script"
grep -Fq '"AXStandardWindow"' "$script"
grep -Fq '"AXCloseButton"' "$script"
if grep -Fq 'set value of attribute "AXFullScreen" of window 1 to true' "$script"; then
  printf 'capture script unexpectedly enables native full-screen\n' >&2
  exit 1
fi

printf 'UI screenshot capture argument contract passed\n'

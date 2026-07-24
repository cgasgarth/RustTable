#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
script="$root/scripts/macos-computer-use-smoke.sh"
package_json="$root/package.json"
help="$("$script" --help)"
[[ "$help" == *"default validates the installation without launching"* ]]
[[ "$help" == *"--allow-foreground"* ]]
[[ "$help" == *"send real Command-Q"* ]]

unknown_error="$("$script" --unknown 2>&1 || true)"
[[ "$unknown_error" == *"unknown macOS computer-use smoke option: --unknown"* ]]
if [[ "$unknown_error" == *"requires macOS"* ]]; then
  printf 'unknown option was not rejected before platform checks\n' >&2
  exit 1
fi

mixed_error="$("$script" --allow-foreground --unknown 2>&1 || true)"
[[ "$mixed_error" == *"unknown macOS computer-use smoke option: --unknown"* ]]

background_body="$(sed -n '/^run_background_smoke()/,/^}/p' "$script")"
foreground_body="$(sed -n '/^run_foreground_command_q_smoke()/,/^}/p' "$script")"
[[ "$background_body" != *'open '* ]]
[[ "$background_body" == *'assert_not_frontmost validated'* ]]
[[ "$background_body" != *'tell application id'* ]]
[[ "$background_body" != *'to activate'* ]]
[[ "$background_body" != *'keystroke "q"'* ]]

[[ "$foreground_body" == *'open "$bundle"'* ]]
[[ "$foreground_body" == *'to activate'* ]]
[[ "$foreground_body" == *'keystroke "q" using command down'* ]]
[[ "$(<"$script")" == *'if [[ "$allow_foreground" == true ]]; then'* ]]
[[ "$(<"$script")" == *'installer_args=(--compact --no-launch)'* ]]

package_contract="$(<"$package_json")"
[[ "$package_contract" == *'"test:macos-computer-use-smoke": "bash scripts/test-macos-computer-use-smoke.sh"'* ]]
[[ "$package_contract" == *'"smoke:macos-computer-use": "bash scripts/macos-computer-use-smoke.sh"'* ]]

printf 'macOS computer-use smoke foreground contract passed\n'

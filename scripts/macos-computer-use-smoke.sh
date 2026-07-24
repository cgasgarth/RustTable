#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bun run smoke:macos-computer-use -- [options]

Build, install, and validate the canonical macOS Computer Use app lifecycle.
The default launches hidden/in the background, never activates RustTable, and
quits through a non-activating application request.

Options:
  --allow-foreground  Activate RustTable and send real Command-Q
  --no-build          Reuse the existing release bundle
  --no-install        Validate the existing canonical installation
  --compact           Accepted for compatibility; output is already compact
  -h, --help          Show this help
EOF
}

allow_foreground=false
installer_args=(--compact --no-launch)
while (($#)); do
  case "$1" in
    --allow-foreground)
      allow_foreground=true
      shift
      ;;
    --no-build|--no-install)
      installer_args+=("$1")
      shift
      ;;
    --compact)
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'unknown macOS computer-use smoke option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != Darwin ]]; then
  printf 'macOS computer-use smoke requires macOS\n' >&2
  exit 2
fi

for tool in git bun open osascript plutil pgrep awk sed sort tr wc sleep; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'required tool is missing: %s\n' "$tool" >&2
    exit 2
  }
done

root="$(git rev-parse --show-toplevel)"
bundle="$HOME/Applications/rusttable - latest.app"
bundle_identifier='com.cgasgarth.rusttable.latest'
lsregister='/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'

[[ -x "$lsregister" ]] || {
  printf 'LaunchServices registry is unavailable\n' >&2
  exit 2
}

install() {
  bun run install:computer-use -- "${installer_args[@]}"
}

install

[[ -x "$bundle/Contents/MacOS/RustTable" ]] || {
  printf 'canonical computer-use executable is missing\n' >&2
  exit 1
}
[[ "$(plutil -extract CFBundleIconFile raw -o - "$bundle/Contents/Info.plist")" == RustTable.icns ]] || {
  printf 'canonical computer-use icon metadata is missing or incorrect\n' >&2
  exit 1
}
[[ -f "$bundle/Contents/Resources/RustTable.icns" ]] || {
  printf 'canonical computer-use icon payload is missing\n' >&2
  exit 1
}

registrations="$($lsregister -dump | awk -v wanted="$bundle_identifier" '
  /^path:[[:space:]]/ {
    path = $0
    sub(/^path:[[:space:]]+/, "", path)
    sub(/[[:space:]]+\(0x[[:xdigit:]]+\)$/, "", path)
  }
  /^identifier:[[:space:]]/ && $2 == wanted {
    print path
  }
')"
unique_registrations="$(printf '%s\n' "$registrations" | sed '/^$/d' | sort -u)"
registration_count="$(printf '%s\n' "$unique_registrations" | sed '/^$/d' | wc -l | tr -d ' ')"
if [[ "$registration_count" != 1 || "$unique_registrations" != "$bundle" ]]; then
  printf 'expected one canonical LaunchServices registration, found:\n%s\n' "$unique_registrations" >&2
  exit 1
fi

process_running() {
  pgrep -f "$bundle/Contents/MacOS/RustTable" >/dev/null 2>&1
}

frontmost_bundle() {
  osascript -l JavaScript -e \
    "const p=Application('System Events').applicationProcesses.whose({frontmost:true})(); p.length ? p[0].bundleIdentifier() : '<none>';"
}

assert_not_frontmost() {
  local stage="$1"
  local frontmost
  frontmost="$(frontmost_bundle)"
  [[ "$frontmost" != "$bundle_identifier" ]] || {
    printf 'RustTable became frontmost during background smoke (%s)\n' "$stage" >&2
    exit 1
  }
}

wait_for_launch() {
  local require_background="$1"
  for _ in {1..40}; do
    [[ "$require_background" != true ]] || assert_not_frontmost launch
    process_running && return
    sleep 0.125
  done
  printf 'installed RustTable did not launch\n' >&2
  exit 1
}

wait_for_exit() {
  local failure="$1"
  local require_background="$2"
  for _ in {1..40}; do
    [[ "$require_background" != true ]] || assert_not_frontmost quit
    process_running || return 0
    sleep 0.125
  done
  printf '%s\n' "$failure" >&2
  exit 1
}

run_background_smoke() {
  open -g -j "$bundle"
  wait_for_launch true
  assert_not_frontmost launched
  # The target can close its AppleEvent connection before `osascript` receives
  # the quit reply. Process exit below is the authoritative lifecycle receipt.
  osascript -e "tell application id \"$bundle_identifier\" to quit" >/dev/null 2>&1 || :
  wait_for_exit 'non-activating quit did not terminate the installed RustTable bundle' true
  assert_not_frontmost exited
  printf 'computer-use background install smoke passed\n'
}

run_foreground_command_q_smoke() {
  open "$bundle"
  wait_for_launch false
  osascript -e "tell application id \"$bundle_identifier\" to activate" \
    -e 'tell application "System Events" to keystroke "q" using command down'
  wait_for_exit 'Command-Q did not terminate the installed RustTable bundle' false
  printf 'computer-use foreground Command-Q smoke passed\n'
}

if [[ "$allow_foreground" == true ]]; then
  run_foreground_command_q_smoke
else
  run_background_smoke
fi

#!/usr/bin/env bash
set -euo pipefail

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
  bun run install:computer-use -- --compact --no-launch "$@"
}

install
install --no-build

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

open "$bundle"
for _ in {1..20}; do
  pgrep -f "$bundle/Contents/MacOS/RustTable" >/dev/null 2>&1 && break
  sleep 0.25
done
pgrep -f "$bundle/Contents/MacOS/RustTable" >/dev/null 2>&1 || {
  printf 'installed RustTable did not launch\n' >&2
  exit 1
}

osascript -e "tell application id \"$bundle_identifier\" to activate" \
  -e 'tell application "System Events" to keystroke "q" using command down'
for _ in {1..20}; do
  pgrep -f "$bundle/Contents/MacOS/RustTable" >/dev/null 2>&1 || {
    printf 'computer-use install smoke passed\n'
    exit 0
  }
  sleep 0.25
done

printf 'Command-Q did not terminate the installed RustTable bundle\n' >&2
exit 1

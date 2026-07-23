#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bun run screenshot:ui-review -- [options]

Build, install, and capture deterministic RustTable Lighttable and Darkroom review PNGs.

Options:
  --width PIXELS     Capture width (default: 1280)
  --height PIXELS    Capture height (default: 768)
  --run-id ID        Output directory name (default: UTC timestamp)
  --no-build         Reuse the existing release bundle
  --no-install       Reuse the existing canonical installed app
  -h, --help         Show this help

Output:
  artifacts/ui-review/<run-id>/rusttable-lighttable.png
  artifacts/ui-review/<run-id>/rusttable-darkroom.png
  artifacts/ui-review/<run-id>/manifest.json
EOF
}

width=1280
height=768
run_id=""
no_build=false
no_install=false

while (($#)); do
  case "$1" in
    --width)
      [[ $# -ge 2 ]] || { printf 'error: --width requires a value\n' >&2; exit 2; }
      width="$2"
      shift 2
      ;;
    --height)
      [[ $# -ge 2 ]] || { printf 'error: --height requires a value\n' >&2; exit 2; }
      height="$2"
      shift 2
      ;;
    --run-id)
      [[ $# -ge 2 ]] || { printf 'error: --run-id requires a value\n' >&2; exit 2; }
      run_id="$2"
      shift 2
      ;;
    --no-build)
      no_build=true
      shift
      ;;
    --no-install)
      no_install=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'error: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ "$width" =~ ^[1-9][0-9]*$ ]] || { printf 'error: width must be a positive integer\n' >&2; exit 2; }
[[ "$height" =~ ^[1-9][0-9]*$ ]] || { printf 'error: height must be a positive integer\n' >&2; exit 2; }
if [[ -n "$run_id" && ! "$run_id" =~ ^[A-Za-z0-9._-]+$ ]]; then
  printf 'error: run ID may contain only letters, digits, dot, underscore, and hyphen\n' >&2
  exit 2
fi
[[ "$(uname -s)" == "Darwin" ]] || { printf 'error: UI review capture requires macOS\n' >&2; exit 1; }

for command in bun git open osascript screencapture sips shasum; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'error: required command not found: %s\n' "$command" >&2
    exit 1
  }
done

root="$(git rev-parse --show-toplevel)"
cd "$root"
app_bundle="$HOME/Applications/rusttable - latest.app"
installer=(bun run install:computer-use --compact --no-launch)
"$no_build" && installer+=(--no-build)
"$no_install" && installer+=(--no-install)
if ! "$no_build" || ! "$no_install"; then
  "${installer[@]}"
fi

[[ -d "$app_bundle" ]] || {
  printf 'error: canonical app bundle is unavailable: %s\n' "$app_bundle" >&2
  exit 1
}

run_id="${run_id:-$(date -u +%Y%m%dT%H%M%SZ)}"
output_dir="$root/artifacts/ui-review/$run_id"
mkdir -p "$output_dir"
lighttable_png="$output_dir/rusttable-lighttable.png"
darkroom_png="$output_dir/rusttable-darkroom.png"
process_name="rusttable - latest"
bundle_id="com.cgasgarth.rusttable.latest"

open -a "$app_bundle"
osascript <<EOF
tell application id "$bundle_id" to activate
tell application "System Events"
  repeat 80 times
    if exists process "$process_name" then
      tell process "$process_name"
        set frontmost to true
        if exists window 1 then
          set position of window 1 to {0, 0}
          set size of window 1 to {$width, $height}
          return
        end if
      end tell
    end if
    delay 0.1
  end repeat
  error "RustTable window did not become available"
end tell
EOF

capture_stable() {
  local view_key="$1"
  local destination="$2"
  # macOS screencapture rejects dot-prefixed destination basenames.
  local previous="$output_dir/capture-previous.tmp.png"
  local current="$output_dir/capture-current.tmp.png"
  rm -f "$previous" "$current"
  osascript <<EOF
tell application id "$bundle_id" to activate
tell application "System Events"
  tell process "$process_name"
    set frontmost to true
    keystroke "$view_key"
  end tell
end tell
delay 3
EOF
  for _attempt in {1..12}; do
    osascript <<EOF
tell application id "$bundle_id" to activate
delay 0.2
tell application "System Events"
  set frontmost_bundle to bundle identifier of first application process whose frontmost is true
end tell
if frontmost_bundle is not "$bundle_id" then error "RustTable is not the frontmost application"
do shell script "/usr/sbin/screencapture -x -R0,0,$width,$height " & quoted form of "$current"
EOF
    [[ -s "$current" ]] || { printf 'error: empty screenshot for view key %s\n' "$view_key" >&2; exit 1; }
    normalize_capture_dimensions "$current"
    if [[ -f "$previous" ]] && [[ "$(shasum -a 256 "$previous" | cut -d' ' -f1)" == "$(shasum -a 256 "$current" | cut -d' ' -f1)" ]]; then
      mv "$current" "$destination"
      rm -f "$previous"
      return
    fi
    mv "$current" "$previous"
    sleep 1
  done
  printf 'error: %s view did not reach a stable captured frame\n' "$view_key" >&2
  exit 1
}

normalize_capture_dimensions() {
  local capture="$1"
  local actual_width actual_height
  actual_width="$(sips -g pixelWidth "$capture" | awk '/pixelWidth/ {print $2}')"
  actual_height="$(sips -g pixelHeight "$capture" | awk '/pixelHeight/ {print $2}')"
  if [[ "$actual_width" != "$width" || "$actual_height" != "$height" ]]; then
    sips --resampleHeightWidth "$height" "$width" "$capture" >/dev/null
  fi
  actual_width="$(sips -g pixelWidth "$capture" | awk '/pixelWidth/ {print $2}')"
  actual_height="$(sips -g pixelHeight "$capture" | awk '/pixelHeight/ {print $2}')"
  if [[ "$actual_width" != "$width" || "$actual_height" != "$height" ]]; then
    printf 'error: screenshot dimensions are %sx%s, expected %sx%s\n' \
      "$actual_width" "$actual_height" "$width" "$height" >&2
    exit 1
  fi
}

capture_stable l "$lighttable_png"
capture_stable d "$darkroom_png"

normalize_capture_dimensions "$lighttable_png"
normalize_capture_dimensions "$darkroom_png"
cmp -s "$lighttable_png" "$darkroom_png" && {
  printf 'error: Lighttable and Darkroom captures are identical; view switching failed\n' >&2
  exit 1
}

commit="$(git rev-parse HEAD)"
captured_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
dirty=false
[[ -z "$(git status --porcelain)" ]] || dirty=true
cat >"$output_dir/manifest.json" <<EOF
{
  "commit": "$commit",
  "dirty": $dirty,
  "dimensions": {"width": $width, "height": $height},
  "app_bundle": "$app_bundle",
  "captured_at": "$captured_at"
}
EOF

printf 'Captured UI review artifacts in %s\n' "$output_dir"

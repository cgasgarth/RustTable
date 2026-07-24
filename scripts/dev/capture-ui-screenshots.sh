#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bun run screenshot:ui-review -- [options]

Build, install, and capture RustTable/Darktable Lighttable and Darkroom review PNGs.

Options:
  --allow-foreground Acknowledge that this command activates and switches apps
  --width PIXELS     Capture width (default: current screen usable width)
  --height PIXELS    Capture height (default: current screen usable height)
  --run-id ID        Output directory name (default: UTC timestamp)
  --reference-app PATH
                     Original Darktable bundle (default: /Applications/darktable.app)
  --reference-dir PATH
                     Reused Darktable baseline directory (default: artifacts/ui-reference)
  --refresh-reference
                     Recapture the Darktable baseline explicitly
  --no-build         Reuse the existing release bundle
  --no-install       Reuse the existing canonical installed app
  -h, --help         Show this help

Output:
  artifacts/ui-review/<run-id>/{rusttable,darktable}-{lighttable,darkroom}.png
  artifacts/ui-review/<run-id>/manifest.json
EOF
}

width=""
height=""
run_id=""
reference_app="/Applications/darktable.app"
reference_dir=""
refresh_reference=false
no_build=false
no_install=false
allow_foreground=false

while (($#)); do
  case "$1" in
    --allow-foreground)
      allow_foreground=true
      shift
      ;;
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
    --reference-app)
      [[ $# -ge 2 ]] || { printf 'error: --reference-app requires a value\n' >&2; exit 2; }
      reference_app="$2"
      shift 2
      ;;
    --reference-dir)
      [[ $# -ge 2 ]] || { printf 'error: --reference-dir requires a value\n' >&2; exit 2; }
      reference_dir="$2"
      shift 2
      ;;
    --refresh-reference)
      refresh_reference=true
      shift
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

if [[ -n "$width" || -n "$height" ]]; then
  [[ -n "$width" && -n "$height" ]] || {
    printf 'error: --width and --height must be supplied together\n' >&2
    exit 2
  }
  [[ "$width" =~ ^[1-9][0-9]*$ ]] || { printf 'error: width must be a positive integer\n' >&2; exit 2; }
  [[ "$height" =~ ^[1-9][0-9]*$ ]] || { printf 'error: height must be a positive integer\n' >&2; exit 2; }
fi
if [[ -n "$run_id" && ! "$run_id" =~ ^[A-Za-z0-9._-]+$ ]]; then
  printf 'error: run ID may contain only letters, digits, dot, underscore, and hyphen\n' >&2
  exit 2
fi
[[ "$allow_foreground" == true ]] || {
  printf 'error: UI review activates and switches foreground apps; rerun with --allow-foreground\n' >&2
  exit 2
}
[[ "$(uname -s)" == "Darwin" ]] || { printf 'error: UI review capture requires macOS\n' >&2; exit 1; }

for command in bun git open osascript screencapture sips shasum; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'error: required command not found: %s\n' "$command" >&2
    exit 1
  }
done

working_area="$(
  osascript -l JavaScript <<'JXA'
ObjC.import('AppKit');
const screen = $.NSScreen.mainScreen;
const frame = screen.frame;
const visible = screen.visibleFrame;
const left = Math.round(Number(visible.origin.x));
const top = Math.round(Number(frame.size.height - visible.origin.y - visible.size.height));
const width = Math.round(Number(visible.size.width));
const height = Math.round(Number(visible.size.height));
`${left} ${top} ${width} ${height}`;
JXA
)"
read -r capture_x capture_y working_width working_height <<<"$working_area"
for value in "$capture_x" "$capture_y" "$working_width" "$working_height"; do
  [[ "$value" =~ ^-?[0-9]+$ ]] || {
    printf 'error: failed to resolve the current macOS usable working area\n' >&2
    exit 1
  }
done
if [[ -z "$width" ]]; then
  width="$working_width"
  height="$working_height"
elif (( width > working_width || height > working_height )); then
  printf 'error: requested %sx%s capture exceeds usable working area %sx%s\n' \
    "$width" "$height" "$working_width" "$working_height" >&2
  exit 2
fi

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
reference_dir="${reference_dir:-$root/artifacts/ui-reference}/${width}x${height}"
lighttable_png="$output_dir/rusttable-lighttable.png"
darkroom_png="$output_dir/rusttable-darkroom.png"
darktable_lighttable_png="$output_dir/darktable-lighttable.png"
darktable_darkroom_png="$output_dir/darktable-darkroom.png"
reference_lighttable_png="$reference_dir/darktable-lighttable.png"
reference_darkroom_png="$reference_dir/darktable-darkroom.png"
reference_captured=false
rusttable_process_name="rusttable - latest"
rusttable_bundle_id="com.cgasgarth.rusttable.latest"
darktable_process_name="darktable"
darktable_bundle_id="org.darktable"

prepare_app() {
  local app_bundle="$1"
  local bundle_id="$2"
  local process_name="$3"
  local app_label="$4"
  open -a "$app_bundle"
  osascript <<EOF
tell application id "$bundle_id" to activate
tell application "System Events"
  repeat 80 times
    if exists process "$process_name" then
      tell process "$process_name"
        set frontmost to true
        if exists window 1 then
          if exists attribute "AXFullScreen" of window 1 then
            set value of attribute "AXFullScreen" of window 1 to false
            repeat 80 times
              if value of attribute "AXFullScreen" of window 1 is false then exit repeat
              delay 0.05
            end repeat
            if value of attribute "AXFullScreen" of window 1 is true then
              error "$app_label remained in native macOS full-screen mode"
            end if
          end if
          if value of attribute "AXSubrole" of window 1 is not "AXStandardWindow" then
            error "$app_label is not a standard decorated macOS window"
          end if
          if not (exists (first button of window 1 whose subrole is "AXCloseButton")) then
            error "$app_label window is missing macOS traffic lights"
          end if
          set position of window 1 to {$capture_x, $capture_y}
          set size of window 1 to {$width, $height}
          return
        end if
      end tell
    end if
    delay 0.1
  end repeat
  error "$app_label window did not become available"
end tell
EOF
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

prepare_app "$app_bundle" "$rusttable_bundle_id" "$rusttable_process_name" RustTable

capture_stable() {
  local app_bundle="$1"
  local bundle_id="$2"
  local process_name="$3"
  local view_key="$4"
  local destination="$5"
  local app_label="$6"
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
if frontmost_bundle is not "$bundle_id" then error "$app_label is not the frontmost application"
do shell script "/usr/sbin/screencapture -x -R$capture_x,$capture_y,$width,$height " & quoted form of "$current"
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
  printf 'error: %s %s view did not reach a stable captured frame\n' "$app_label" "$view_key" >&2
  exit 1
}

capture_stable "$app_bundle" "$rusttable_bundle_id" "$rusttable_process_name" l "$lighttable_png" RustTable
capture_stable "$app_bundle" "$rusttable_bundle_id" "$rusttable_process_name" d "$darkroom_png" RustTable

if [[ "$refresh_reference" == true || ! -s "$reference_lighttable_png" || ! -s "$reference_darkroom_png" ]]; then
  [[ -d "$reference_app" ]] || {
    printf 'error: reference Darktable app is unavailable: %s\n' "$reference_app" >&2
    exit 1
  }
  mkdir -p "$reference_dir"
  prepare_app "$reference_app" "$darktable_bundle_id" "$darktable_process_name" Darktable
  capture_stable "$reference_app" "$darktable_bundle_id" "$darktable_process_name" l "$reference_lighttable_png" Darktable
  capture_stable "$reference_app" "$darktable_bundle_id" "$darktable_process_name" d "$reference_darkroom_png" Darktable
  reference_captured=true
fi
cp "$reference_lighttable_png" "$darktable_lighttable_png"
cp "$reference_darkroom_png" "$darktable_darkroom_png"

normalize_capture_dimensions "$lighttable_png"
normalize_capture_dimensions "$darkroom_png"
normalize_capture_dimensions "$darktable_lighttable_png"
normalize_capture_dimensions "$darktable_darkroom_png"
cmp -s "$lighttable_png" "$darkroom_png" && {
  printf 'error: Lighttable and Darkroom captures are identical; view switching failed\n' >&2
  exit 1
}
cmp -s "$darktable_lighttable_png" "$darktable_darkroom_png" && {
  printf 'error: Darktable Lighttable and Darkroom captures are identical; view switching failed\n' >&2
  exit 1
}

commit="$(git rev-parse HEAD)"
captured_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
dirty=false
[[ -z "$(git status --porcelain)" ]] || dirty=true
sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
cat >"$output_dir/manifest.json" <<EOF
{
  "commit": "$commit",
  "dirty": $dirty,
  "dimensions": {"width": $width, "height": $height},
  "working_area": {"x": $capture_x, "y": $capture_y, "width": $working_width, "height": $working_height},
  "app_bundle": "$app_bundle",
  "reference_app": "$reference_app",
  "reference_dir": "$reference_dir",
  "reference_captured": $reference_captured,
  "reference_refreshed": $refresh_reference,
  "captures": {
    "rusttable": {
      "lighttable": {"path": "rusttable-lighttable.png", "sha256": "$(sha256 "$lighttable_png")"},
      "darkroom": {"path": "rusttable-darkroom.png", "sha256": "$(sha256 "$darkroom_png")"}
    },
    "darktable": {
      "lighttable": {"path": "darktable-lighttable.png", "sha256": "$(sha256 "$darktable_lighttable_png")"},
      "darkroom": {"path": "darktable-darkroom.png", "sha256": "$(sha256 "$darktable_darkroom_png")"}
    }
  },
  "captured_at": "$captured_at"
}
EOF

printf 'Captured UI review artifacts in %s\n' "$output_dir"

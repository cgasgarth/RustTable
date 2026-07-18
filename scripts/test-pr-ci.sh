#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

write_fake_tools() {
  local directory="$1"
  cat >"$directory/git" <<'EOF'
#!/bin/sh
if [ "${1:-} ${2:-}" = "diff --check" ] && case ",${FAKE_FAILS:-}," in *,diff,*) true;; *) false;; esac; then
  echo "fake diff failure"
  exit 11
fi
exit 0
EOF
  cat >"$directory/cargo" <<'EOF'
#!/bin/sh
case "${1:-}" in
  fmt) label=fmt ;;
  metadata) label=metadata ;;
  clippy) label=clippy ;;
  test) label=test ;;
  *) label=other ;;
esac
if [ "${RUSTTABLE_LAYOUT_CHECK:-0}" = 1 ] && [ "$label" = metadata ]; then
  label=workspace-layout
fi
case ",${FAKE_FAILS:-}," in *",$label,"*)
  echo "fake $label failure"
  exit 12
  ;;
esac
if [ "$label" = workspace-layout ]; then
  printf '%s\n' '{"workspace_members":["app","catalog","catalog-store","core","diagnostics","image","image-io","import","metadata","processing","render","ui"],"packages":[{"id":"app","name":"rusttable-app","dependencies":[{"name":"rusttable-ui","kind":null}]},{"id":"catalog","name":"rusttable-catalog","dependencies":[]},{"id":"catalog-store","name":"rusttable-catalog-store","dependencies":[]},{"id":"core","name":"rusttable-core","dependencies":[]},{"id":"diagnostics","name":"rusttable-diagnostics","dependencies":[]},{"id":"image","name":"rusttable-image","dependencies":[]},{"id":"image-io","name":"rusttable-image-io","dependencies":[]},{"id":"import","name":"rusttable-import","dependencies":[]},{"id":"metadata","name":"rusttable-metadata","dependencies":[]},{"id":"processing","name":"rusttable-processing","dependencies":[]},{"id":"render","name":"rusttable-render","dependencies":[]},{"id":"ui","name":"rusttable-ui","dependencies":[{"name":"rusttable-core","kind":null},{"name":"iced","kind":null}]}]}'
fi
if [ "$label" = clippy ] || [ "$label" = test ]; then
  printf '%s\n' "$label" >>"$FAKE_MARKERS"
fi
exit 0
EOF
  cat >"$directory/bun" <<'EOF'
#!/bin/sh
label=bun
  case "$*" in
    *workspace-rust-version*) label=workspace-rust-version ;;
    *workspace-layout*) label=workspace-layout ;;
    *macos-artifact-identity*) label=macos-artifact-identity ;;
  esac
case ",${FAKE_FAILS:-}," in *",$label,"*)
  echo "fake $label failure"
  exit 13
  ;;
esac
exit 0
EOF
  cat >"$directory/bash" <<'EOF'
#!/bin/sh
if [ "${1:-}" = scripts/check-source-policy.sh ] && case ",${FAKE_FAILS:-}," in *,source,*) true;; *) false;; esac; then
  echo "fake source failure"
  exit 14
fi
exec /bin/bash "$@"
EOF
  chmod +x "$directory"/{git,cargo,bun,bash}
}

assert_contains() {
  local needle="$1"
  local file="$2"
  grep -F "$needle" "$file" >/dev/null
}

run_pr_ci() {
  local behavior="$1"
  local output="$2"
  : >"$FAKE_MARKERS"
  local status=0
  if FAKE_FAILS="$behavior" RUSTTABLE_SKIP_BUN_PIN_REGRESSION=1 RUSTTABLE_SKIP_PR_CI_REGRESSION=1 PATH="$fake_tools:$PATH" \
    /bin/bash "$root_directory/scripts/pr-ci.sh" >"$output" 2>&1; then
    status=0
  else
    status="$?"
  fi
  return "$status"
}

fake_tools="$temporary_directory/tools"
mkdir -p "$fake_tools"
write_fake_tools "$fake_tools"
FAKE_MARKERS="$temporary_directory/markers"
export FAKE_MARKERS

for label in diff fmt metadata source bun macos-artifact-identity workspace-rust-version workspace-layout; do
  output="$temporary_directory/$label.log"
  if [ "$label" = workspace-layout ]; then
    export RUSTTABLE_LAYOUT_CHECK=1
  else
    unset RUSTTABLE_LAYOUT_CHECK
  fi
  if run_pr_ci "$label" "$output"; then
    echo "expected cheap check failure: $label" >&2
    exit 1
  fi
  assert_contains "PR check failed: $label" "$output"
  assert_contains "fake $label failure" "$output"
  [[ ! -s "$FAKE_MARKERS" ]]
done

output="$temporary_directory/simultaneous.log"
if run_pr_ci diff,fmt "$output"; then
  echo "expected simultaneous cheap check failure" >&2
  exit 1
fi
assert_contains "PR check failed: diff" "$output"
assert_contains "PR check failed: fmt" "$output"
[[ ! -s "$FAKE_MARKERS" ]]

output="$temporary_directory/clean.log"
if ! run_pr_ci "" "$output"; then
  cat "$output" >&2
  exit 1
fi
[[ "$(grep -c '^clippy$' "$FAKE_MARKERS")" == 1 ]]
[[ "$(grep -c '^test$' "$FAKE_MARKERS")" == 1 ]]
[[ "$(sed -n '1p' "$FAKE_MARKERS")" == clippy ]]
[[ "$(sed -n '2p' "$FAKE_MARKERS")" == test ]]

for label in clippy test; do
  output="$temporary_directory/$label-heavy.log"
  if run_pr_ci "$label" "$output"; then
    echo "expected heavyweight check failure: $label" >&2
    exit 1
  fi
  assert_contains "fake $label failure" "$output"
done

echo "pr-ci regression fixtures passed"

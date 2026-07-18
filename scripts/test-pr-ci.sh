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
case ",${FAKE_FAILS:-}," in *",$label,"*)
  echo "fake $label failure"
  exit 12
  ;;
esac
if [ "$label" = clippy ] || [ "$label" = test ]; then
  printf '%s\n' "$label" >>"$FAKE_MARKERS"
fi
exit 0
EOF
  cat >"$directory/bun" <<'EOF'
#!/bin/sh
case ",${FAKE_FAILS:-}," in *,bun,*)
  echo "fake bun failure"
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
  if FAKE_FAILS="$behavior" RUSTTABLE_SKIP_PR_CI_REGRESSION=1 PATH="$fake_tools:$PATH" \
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

for label in diff fmt metadata source bun; do
  output="$temporary_directory/$label.log"
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

#!/usr/bin/env bash
set -euo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR

root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
doctor="$root/scripts/dev/doctor.sh"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

repo="$fixture/repo"
bin="$fixture/bin"
mkdir -p "$repo/.git" "$repo/.githooks" "$repo/scripts/dev" "$bin"
cp "$doctor" "$repo/scripts/dev/doctor.sh"
chmod +x "$repo/scripts/dev/doctor.sh"
touch "$repo/.githooks/pre-commit" "$repo/.githooks/pre-push"
chmod +x "$repo/.githooks/pre-commit" "$repo/.githooks/pre-push"
printf '[toolchain]\nchannel = "fixture-toolchain"\ncomponents = ["clippy", "rustfmt"]\n' >"$repo/rust-toolchain.toml"
printf '{"packageManager":"bun@fixture-bun"}\n' >"$repo/package.json"
touch "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/TASK.md" "$repo/AGENTS.md"

cat >"$bin/git" <<'EOF'
#!/usr/bin/env bash
case "$1 $2 $3" in
  'rev-parse --show-toplevel ') printf '%s\n' "$FAKE_ROOT" ;;
  'rev-parse --is-inside-work-tree ') printf 'true\n' ;;
  'symbolic-ref --quiet --short') printf '%s\n' "${FAKE_BRANCH:-feature/doctor}" ;;
  'config --get core.hooksPath') printf '.githooks\n' ;;
  'remote get-url origin') printf '%s\n' "${FAKE_ORIGIN:-https://github.com/cgasgarth/RustTable.git}" ;;
  'remote get-url upstream') printf 'git@github.com:cgasgarth/RustTable.git\n' ;;
  'remote  ') printf 'origin\nupstream\n' ;;
  ls-files\ --error-unmatch*) exit 0 ;;
esac
if [[ "$1 $2" == 'config --get-all' ]]; then
  exit 0
fi
exit 0
EOF

cat >"$bin/rustup" <<'EOF'
#!/usr/bin/env bash
if [[ "$1 $2" == 'show active-toolchain' ]]; then
  printf '%s\n' "${FAKE_RUSTUP_TOOLCHAIN:-fixture-toolchain} (default)"
elif [[ "$1 $2 $3" == 'component list --installed' ]]; then
  printf 'clippy-x86_64-unknown-linux-gnu (installed)\nrustfmt-x86_64-unknown-linux-gnu (installed)\n'
fi
EOF
cat >"$bin/bun" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${FAKE_BUN_VERSION:-fixture-bun}"
EOF
for tool in cargo rustc rustfmt cargo-clippy pre-commit rg; do
  printf '#!/usr/bin/env bash\nexit 0\n' >"$bin/$tool"
done
chmod +x "$bin"/*
fixture_path="$bin:/usr/bin:/bin"

run_doctor() {
  (cd "$repo" && FAKE_ROOT="$repo" env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE PATH="$fixture_path" bash scripts/dev/doctor.sh)
}

if ! run_doctor >"$fixture/compliant-output" 2>&1; then
  cat "$fixture/compliant-output" >&2
  exit 1
fi

if (cd "$repo" && FAKE_ROOT="$repo" FAKE_BUN_VERSION=wrong-bun env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE PATH="$fixture_path" bash scripts/dev/doctor.sh >"$fixture/output" 2>&1); then
  printf 'expected Bun mismatch to fail\n' >&2
  exit 1
fi
grep -q 'Bun version does not match package.json' "$fixture/output"

if (cd "$repo" && FAKE_ROOT="$repo" FAKE_BRANCH=main env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE PATH="$fixture_path" bash scripts/dev/doctor.sh >"$fixture/output" 2>&1); then
  printf 'expected protected branch to fail\n' >&2
  exit 1
fi
grep -q 'implementation branch is protected: main' "$fixture/output"
if (cd "$repo" && FAKE_ROOT="$repo" FAKE_ORIGIN=https://user:secret@example.invalid/RustTable.git rm "$bin/pre-commit" && FAKE_ROOT="$repo" FAKE_ORIGIN=https://user:secret@example.invalid/RustTable.git env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE PATH="$fixture_path" bash scripts/dev/doctor.sh >"$fixture/output" 2>&1); then
  printf 'expected aggregated failures to fail\n' >&2
  exit 1
fi
grep -q 'origin must resolve to cgasgarth/RustTable' "$fixture/output"
grep -q 'missing tool: pre-commit' "$fixture/output"
if grep -q secret "$fixture/output"; then
  printf 'doctor leaked remote credentials\n' >&2
  exit 1
fi

printf 'doctor fixtures: PASS\n'

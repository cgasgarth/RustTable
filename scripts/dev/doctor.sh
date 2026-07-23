#!/usr/bin/env bash
set -u

root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
failures=0

fail() {
  printf 'FAIL: %s\n' "$1"
  failures=$((failures + 1))
}

pass() {
  printf 'PASS: %s\n' "$1"
}

if [[ -z "$root" ]] || [[ ! -d "$root/.git" && ! -f "$root/.git" ]]; then
  fail 'current directory is not a RustTable Git worktree'
  printf 'RustTable doctor: %s failure(s)\n' "$failures"
  exit 1
fi

if [[ "$(git rev-parse --is-inside-work-tree 2>/dev/null || true)" == true ]]; then
  pass 'current directory is a Git worktree'
else
  fail 'current directory is not a Git worktree'
fi

branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
case "$branch" in
  main|master) fail "implementation branch is protected: $branch" ;;
  '') fail 'current branch is detached' ;;
  *) pass 'implementation branch is not protected' ;;
esac

hooks_path="$(git config --get core.hooksPath 2>/dev/null || true)"
if [[ "$hooks_path" == .githooks || "$hooks_path" == "$root/.githooks" ]]; then
  pass 'core.hooksPath resolves to .githooks'
else
  fail 'core.hooksPath must resolve to .githooks'
fi

for hook in pre-commit pre-push; do
  if [[ -x "$root/.githooks/$hook" ]] && git ls-files --error-unmatch ".githooks/$hook" >/dev/null 2>&1; then
    pass "tracked executable hook exists: $hook"
  elif [[ ! -e "$root/.githooks/$hook" ]]; then
    fail "missing hook: $hook"
  else
    fail "hook is not tracked and executable: $hook"
  fi
done

for tool in git rustup cargo rustc rustfmt cargo-clippy cargo-deny bun rg; do
  if command -v "$tool" >/dev/null 2>&1; then
    pass "tool available: $tool"
  else
    fail "missing tool: $tool"
  fi
done

toolchain_channel="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$root/rust-toolchain.toml" 2>/dev/null | head -n 1)"
if [[ -z "$toolchain_channel" ]]; then
  fail 'rust-toolchain.toml has no canonical channel'
else
  active_toolchain="$(rustup show active-toolchain 2>/dev/null || true)"
  case "$active_toolchain" in
    "$toolchain_channel"\ *) pass 'active Rust toolchain matches rust-toolchain.toml' ;;
    "$toolchain_channel") pass 'active Rust toolchain matches rust-toolchain.toml' ;;
    *) fail 'active Rust toolchain does not match rust-toolchain.toml' ;;
  esac
  components="$(sed -n 's/^components = \[\(.*\)\]/\1/p' "$root/rust-toolchain.toml" 2>/dev/null | tr -d '" ' | tr ',' '\n')"
  for component in $components; do
    if rustup component list --installed 2>/dev/null | grep -q "^$component.*(installed)"; then
      pass "Rust component installed: $component"
    else
      fail "Rust component missing: $component"
    fi
  done
fi

package_manager="$(sed -n 's/.*"packageManager"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$root/package.json" 2>/dev/null | head -n 1)"
if [[ "$package_manager" == bun@* ]]; then
  bun_version="$(bun --version 2>/dev/null || true)"
  if [[ "bun@$bun_version" == "$package_manager" ]]; then
    pass 'Bun version matches package.json'
  else
    fail 'Bun version does not match package.json'
  fi
else
  fail 'package.json has no canonical Bun packageManager'
fi

remote_matches_rusttable() {
  case "$1" in
    https://github.com/cgasgarth/RustTable|https://github.com/cgasgarth/RustTable.git|git@github.com:cgasgarth/RustTable|git@github.com:cgasgarth/RustTable.git) return 0 ;;
    *) return 1 ;;
  esac
}

remote_url="$(git remote get-url origin 2>/dev/null || true)"
if remote_matches_rusttable "$remote_url"; then
  pass 'origin resolves to cgasgarth/RustTable'
else
  fail 'origin must resolve to cgasgarth/RustTable'
fi

for remote in $(git remote 2>/dev/null); do
  push_urls="$(git config --get-all "remote.$remote.pushurl" 2>/dev/null || true)"
  for push_url in $push_urls; do
    case "$push_url" in
      *darktable-org/darktable*) fail "push URL is forbidden for remote: $remote" ;;
    esac
  done
done
pass 'no configured push URL targets darktable-org/darktable'

for required in Cargo.toml Cargo.lock rust-toolchain.toml package.json TASK.md AGENTS.md; do
  if git ls-files --error-unmatch "$required" >/dev/null 2>&1; then
    pass "tracked repository file exists: $required"
  else
    fail "missing tracked repository file: $required"
  fi
done

if (( failures == 0 )); then
  printf 'RustTable doctor: PASS\n'
  exit 0
fi

printf 'RustTable doctor: %s failure(s)\n' "$failures"
printf 'Remediation: activate hooks with git config core.hooksPath .githooks; use a feature branch and the canonical toolchain files.\n'
exit 1

#!/usr/bin/env bash
set -euo pipefail

source_root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
script="$source_root/scripts/dev/create-agent-worktree.sh"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

git -C "$fixture" init -q -b main
git -C "$fixture" config user.email test@example.invalid
git -C "$fixture" config user.name 'RustTable test'
mkdir -p "$fixture/scripts/dev" "$fixture/source"
cp "$script" "$fixture/scripts/dev/create-agent-worktree.sh"
chmod +x "$fixture/scripts/dev/create-agent-worktree.sh"
printf '%s\n' fixture >"$fixture/README"
git -C "$fixture" add README scripts
git -C "$fixture" commit -qm initial
git -C "$fixture" remote add origin "$fixture"
printf '%s\n' local >"$fixture/source/needed.txt"

default_fixture="$fixture/management/RustTable"
mkdir -p "$default_fixture/scripts/dev"
cp "$script" "$default_fixture/scripts/dev/create-agent-worktree.sh"
chmod +x "$default_fixture/scripts/dev/create-agent-worktree.sh"
printf '%s\n' fixture >"$default_fixture/README"
git -C "$default_fixture" init -q -b main
git -C "$default_fixture" config user.email test@example.invalid
git -C "$default_fixture" config user.name 'RustTable test'
git -C "$default_fixture" add README scripts
git -C "$default_fixture" commit -qm initial
git -C "$default_fixture" remote add origin "$default_fixture"

(cd "$default_fixture" && bash scripts/dev/create-agent-worktree.sh --issue 97 >/dev/null)
default_target="$fixture/management/worktrees/issue-97"
[[ "$(git -C "$default_target" branch --show-current)" == codex/issue-97-agent ]]
[[ ! -e "$default_fixture/worktrees/issue-97" ]]

worktrees="$fixture/worktrees"
(cd "$fixture" && bash scripts/dev/create-agent-worktree.sh --issue 98 --worktrees "$worktrees" >/dev/null)
empty_target="$worktrees/issue-98"
[[ "$(git -C "$empty_target" branch --show-current)" == codex/issue-98-agent ]]
[[ ! -e "$empty_target/source/needed.txt" ]]

(cd "$fixture" && bash scripts/dev/create-agent-worktree.sh --issue 99 --worktrees "$worktrees" --include source/needed.txt >/dev/null)
target="$worktrees/issue-99"
[[ "$(git -C "$target" branch --show-current)" == codex/issue-99-agent ]]
[[ "$(<"$target/source/needed.txt")" == local ]]

(cd "$fixture" && bash scripts/dev/create-agent-worktree.sh --issue 99 --worktrees "$worktrees" --include source/needed.txt >/dev/null)
if (cd "$fixture" && bash scripts/dev/create-agent-worktree.sh --issue 100 --worktrees "$worktrees" --include README >/dev/null 2>&1); then
  printf 'tracked include unexpectedly passed\n' >&2
  exit 1
fi

printf 'agent worktree: tests PASS\n'

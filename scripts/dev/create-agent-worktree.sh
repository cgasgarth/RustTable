#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/dev/create-agent-worktree.sh --issue NUMBER [options]

Create or reuse an agent worktree from the latest origin/main.

Options:
  --issue NUMBER       GitHub issue number; defaults the worktree name
  --branch NAME        Branch name; defaults to codex/issue-NUMBER-agent
  --name NAME          Worktree directory name; defaults to issue-NUMBER
  --include PATH       Copy this repository-relative untracked path into the worktree
  --worktrees PATH     Override the canonical worktrees directory
  -h, --help           Show this help
USAGE
}

fail() {
  printf 'agent worktree: %s\n' "$*" >&2
  exit 1
}

source_root="$(git rev-parse --show-toplevel 2>/dev/null)" || fail 'run from a RustTable Git worktree'
issue=''
branch=''
name=''
worktrees=''
includes=()

while (($# > 0)); do
  case "$1" in
    --issue)
      (($# >= 2)) || fail '--issue requires a number'
      issue="$2"
      shift 2
      ;;
    --branch)
      (($# >= 2)) || fail '--branch requires a name'
      branch="$2"
      shift 2
      ;;
    --name)
      (($# >= 2)) || fail '--name requires a directory name'
      name="$2"
      shift 2
      ;;
    --include)
      (($# >= 2)) || fail '--include requires a path'
      includes+=("$2")
      shift 2
      ;;
    --worktrees)
      (($# >= 2)) || fail '--worktrees requires a path'
      worktrees="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

[[ "$issue" =~ ^[0-9]+$ ]] || fail '--issue must be a positive integer'
(( issue > 0 )) || fail '--issue must be a positive integer'
[[ -n "$name" ]] || name="issue-$issue"
[[ -n "$branch" ]] || branch="codex/issue-$issue-agent"
[[ "$name" != */* && "$name" != . && "$name" != .. ]] || fail '--name must be a single directory name'
[[ "$branch" != -* && "$branch" != *'..'* ]] || fail '--branch contains an unsafe path component'
[[ "$branch" != main && "$branch" != master ]] || fail 'agent worktrees cannot use the protected default branch'

if [[ -z "$worktrees" ]]; then
  case "$source_root" in
    */worktrees/*) worktrees="${source_root%%/worktrees/*}/worktrees" ;;
    *) worktrees="$source_root/worktrees" ;;
  esac
fi
worktrees="$(cd "$(dirname "$worktrees")" && pwd)/$(basename "$worktrees")"
target="$worktrees/$name"

git -C "$source_root" diff --quiet || fail 'source worktree has unstaged tracked changes'
git -C "$source_root" diff --cached --quiet || fail 'source worktree has staged tracked changes'
git -C "$source_root" fetch origin main --prune >/dev/null
git -C "$source_root" rev-parse --verify origin/main^{commit} >/dev/null || fail 'origin/main is unavailable'

if [[ -e "$target" ]]; then
  [[ -d "$target" ]] || fail "target exists and is not a directory: $target"
  target_real="$(cd "$target" && pwd)"
  expected_real="$(cd "$worktrees" && pwd)/$name"
  [[ "$target_real" == "$expected_real" ]] || fail 'target path is not canonical'
  actual_branch="$(git -C "$target" branch --show-current 2>/dev/null || true)"
  [[ "$actual_branch" == "$branch" ]] || fail "existing worktree uses branch '$actual_branch', expected '$branch'"
  printf 'agent worktree: reusing %s (origin/main refreshed)\n' "$target"
else
  git -C "$source_root" show-ref --verify --quiet "refs/heads/$branch" && fail "branch already exists: $branch"
  mkdir -p "$worktrees"
  git -C "$source_root" worktree add -b "$branch" "$target" origin/main >/dev/null
  printf 'agent worktree: created %s from origin/main\n' "$target"
fi

copy_path() {
  local relative="$1"
  [[ "$relative" != /* && "$relative" != '.' && "$relative" != '..' && "$relative" != *'..'* ]] || fail "unsafe include path: $relative"
  [[ -e "$source_root/$relative" ]] || fail "include path does not exist: $relative"
  git -C "$source_root" ls-files --error-unmatch -- "$relative" >/dev/null 2>&1 && fail "include path is tracked; omit --include: $relative"

  local destination="$target/$relative"
  mkdir -p "$(dirname "$destination")"
  if [[ -e "$destination" ]]; then
    if [[ -d "$source_root/$relative" ]]; then
      diff -qr "$source_root/$relative" "$destination" >/dev/null || fail "existing include differs: $relative"
    else
      cmp -s "$source_root/$relative" "$destination" || fail "existing include differs: $relative"
    fi
    return
  fi
  cp -R "$source_root/$relative" "$destination"
  printf 'agent worktree: copied untracked %s\n' "$relative"
}

if ((${#includes[@]} > 0)); then
  for relative in "${includes[@]}"; do
    copy_path "$relative"
  done
fi

printf 'agent worktree: ready %s\n' "$target"

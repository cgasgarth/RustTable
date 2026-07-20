# Contributing to RustTable

RustTable is a complete Rust rewrite of darktable. The original C/C++ project
is a read-only behavioral reference, not a source tree to extend or merge
back into. New product work belongs in the Rust workspace and uses Iced for UI
surfaces.

## Start with an issue

Use an existing GitHub issue, or discuss and create one before beginning a
meaningful change. The issue defines scope, acceptance evidence, source-map
accounting where applicable, and the milestone. A ready pull request closes
one issue; do not mix unrelated work into it.

## Local setup

Install the exact Rust toolchain declared by `rust-toolchain.toml` (currently
the dated Rust 1.98 beta baseline) and the Bun version pinned by `package.json`.
Then configure hooks and use an isolated worktree:

```sh
git config core.hooksPath .githooks
bash scripts/dev/create-agent-worktree.sh --issue ISSUE_NUMBER
cd /Users/cgas/Documents/RustTable/worktrees/issue-ISSUE_NUMBER
bash scripts/dev/doctor.sh
```

`origin` and `upstream` point to `cgasgarth/RustTable`; the `darktable` remote
is reference-only. Never push to `darktable-org/darktable`, and never commit
directly to `main`.

## Implementation expectations

- Prefer small, test-driven, deterministic changes. Add a focused regression
  test for a defect and the appropriate unit, integration, or end-to-end
  evidence for behavior changes.
- Use workspace dependencies and workspace lints. Warnings are errors and
  `unsafe_code` is forbidden unless a narrowly justified, documented, and
  tested exception is approved.
- Keep handwritten source files at or below 1,000 lines. Generated files must
  be clearly identified; split handwritten modules before they reach the cap.
- Reuse established Rust crates when they improve the result, otherwise prefer
  standard library and framework facilities. Do not introduce parallel C/C++,
  GTK, CMake, or native build paths into RustTable.

## Validate and submit

Run the complete local merge-readiness gate before every commit and again
before opening a pull request:

```sh
bash scripts/precommit-fast.sh
bash scripts/prepush-fast.sh
```

The checks intentionally have no elapsed-time cap: correctness and early
feedback take precedence over an arbitrary per-command deadline. GitHub
Actions run only after pushes to `main`; they repeat local checks and run the
heavier merge validation.

Open pull requests ready for review, with `Why`, `How`, and `Validation`
sections plus `Closes #ISSUE_NUMBER`. After required review and successful
local validation, enable squash auto-merge. Do not open a draft PR unless the
issue explicitly requires one.

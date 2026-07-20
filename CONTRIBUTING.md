# Contributing to RustTable

RustTable is a Rust and Iced rewrite of darktable. The original native project is reference material, not code to extend or merge into this repository.

## Start from GitHub

Choose an issue with a milestone and priority, then create a dedicated worktree:

```sh
git config core.hooksPath .githooks
bash scripts/dev/create-agent-worktree.sh --issue ISSUE_NUMBER
cd /Users/cgas/Documents/RustTable/worktrees/issue-ISSUE_NUMBER
bash scripts/dev/doctor.sh
```

GitHub is the planning source of truth. Repository tooling does not mirror, hash, schedule, or mutate issue content.

## Implement

- Use the pinned Rust toolchain and workspace dependencies/lints.
- Keep unsafe code forbidden and handwritten files at or below 1,000 lines.
- Write deterministic tests first for behavior changes and defects.
- Prefer Rust/Iced facilities and maintained crates.
- Do not add C/C++, CMake, GTK, or copied OpenCL source.

## Validate and submit

Run the complete local gate:

```sh
cargo xtask check
```

Open one ready-for-review PR with `Why`, `How`, `Validation`, and `Closes #ISSUE_NUMBER`. Pull-request CI is authoritative on Linux, macOS, Windows, and dependency policy. Use squash auto-merge after required checks and review.

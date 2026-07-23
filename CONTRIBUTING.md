# Contributing to RustTable

RustTable is a Rust and GTK4/gtk-rs rewrite of darktable. The original native project is reference material, not code to extend or merge into this repository.

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
- Keep unsafe code forbidden. Treat 1,000 lines as a maintainability trigger, not a hard ceiling: split growing handwritten files when responsibility-based decomposition improves navigability, keep related children under an explicit parent, and allow cohesive files to exceed the guideline when splitting would obscure ownership.
- Write deterministic tests first for behavior changes and defects.
- Prefer Rust, GTK4/GLib facilities, and maintained crates.
- Shift responsibilities in place from Darktable's `src/gui`, `src/libs`, `src/views`, and `src/iop` into coherent Rust GTK4 modules. Preserve workflow and layout behavior, not C APIs or source files.
- Do not add C/C++, CMake, GTK3, or copied OpenCL source.

## Validate and submit

Run the complete local gate:

```sh
cargo xtask check
```

Open one ready-for-review PR with `Why`, `How`, `Validation`, and `Closes #ISSUE_NUMBER`. Pull-request CI is authoritative on Linux, macOS, Windows, and dependency policy. Use squash auto-merge after required checks and review.

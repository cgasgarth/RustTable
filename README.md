# RustTable

RustTable is a complete rewrite of [darktable](https://github.com/darktable-org/darktable) in Rust, with [iced](https://iced.rs/) as the application UI framework. The RustTable repository contains only the active Rust implementation and its validation surface; the original tree is kept separately for reference.

## Status

The migration is being delivered through focused GitHub issues and squash-merged pull requests. The current plan and operating rules are [TASK.md](TASK.md) and [AGENTS.md](AGENTS.md).

## Prerequisites

Install Rust through the toolchain declared in [rust-toolchain.toml](rust-toolchain.toml), and install Bun through the `packageManager` value in [package.json](package.json). The repository doctor checks the complete local setup without installing or fetching anything.

## Clone and worktrees

```sh
git clone https://github.com/cgasgarth/RustTable.git
cd RustTable
git config core.hooksPath .githooks
mkdir -p /Users/cgas/Documents/RustTable/worktrees
git worktree add /Users/cgas/Documents/RustTable/worktrees/<issue> -b codex/<issue> main
bash scripts/dev/doctor.sh
```

The canonical remotes are `origin` and `upstream`, both resolving to `cgasgarth/RustTable`; the `darktable` remote is reference-only. Never push to `darktable-org/darktable`.

## Validation

Run the fast checks before publishing work:

```sh
bash scripts/precommit-fast.sh  # full local build/lint/test gate; intentionally uncapped
bash scripts/prepush-fast.sh    # <= 150 seconds
bash scripts/main-ci.sh          # merge-to-main validation
```

Pre-commit is the complete local merge-readiness gate. GitHub Actions runs from pushes to
`main`, repeating that gate and adding coverage, offline closure, packaging, platform,
security, provenance, and compiler-channel validation.

## Application

Build and run the iced application with locked dependencies:

```sh
cargo build --package rusttable-app --bin rusttable-app --locked
cargo run --package rusttable-app --bin rusttable-app --locked
```

On macOS, Computer Use installs the canonical `rusttable - latest` app at
`~/Applications/rusttable - latest.app`:

```sh
bun run install:computer-use
```

## Contribution workflow

Use one GitHub issue, one dedicated worktree, and one pull request per change. Work on a feature branch, pass the local gates, open a PR against `main`, and squash-merge only after review and the local gate pass. Do not commit directly to `main`; repository policy and detailed migration guidance live in [TASK.md](TASK.md) and [AGENTS.md](AGENTS.md).

## Reference

The original [darktable project](https://github.com/darktable-org/darktable) remains available in the local reference clone at `/Users/cgas/Documents/RustTable/upstream` for historical context and behavioral comparison. RustTable's active source is the workspace described by [Cargo.toml](Cargo.toml), with the iced application in [crates/rusttable-app](crates/rusttable-app).

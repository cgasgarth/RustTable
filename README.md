# RustTable

RustTable is a complete rewrite of [darktable](https://github.com/darktable-org/darktable) in Rust, using [Iced](https://iced.rs/) for the desktop UI. This repository contains the Rust product; the original project is kept in a separate read-only clone for behavioral and format reference.

## Setup

Install the toolchain selected by `rust-toolchain.toml`. Bun is needed only for the macOS app installer and distribution tooling.

```sh
git clone https://github.com/cgasgarth/RustTable.git
cd RustTable
cargo install cargo-deny --version 0.19.8 --locked
git config core.hooksPath .githooks
bash scripts/dev/doctor.sh
```

Create issue worktrees under the dedicated directory:

```sh
bash scripts/dev/create-agent-worktree.sh --issue ISSUE_NUMBER
```

## Build and run

```sh
cargo build --package rusttable-app --bin rusttable-app --locked
cargo run --package rusttable-app --bin rusttable-app --locked
```

On macOS, install or replace the canonical Computer Use app:

```sh
bun run install:computer-use
```

## Product engineering tasks

```sh
cargo xtask check
cargo xtask codegen operations --check
cargo xtask fixtures verify
cargo xtask reference provision --help
cargo xtask reference test --help
cargo xtask bench run --check
cargo xtask bench compare --help
cargo xtask dist
```

`cargo xtask check` runs formatting, strict Clippy, full workspace tests, source policy, generated-operation validation, the real fixture corpus, and standard dependency checks. Pull-request CI repeats merge-authoritative checks on Linux, macOS, and Windows. Coverage and distribution run after merge and for releases.

## Contribution model

GitHub issues, priority labels, and milestones define work. Implement one coherent issue in a dedicated worktree, open a ready-for-review PR against `main`, and squash merge after validation. See [CONTRIBUTING.md](CONTRIBUTING.md), [TASK.md](TASK.md), and [AGENTS.md](AGENTS.md).

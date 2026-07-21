# RustTable Engineering Guidelines

## Product direction

- RustTable is a complete Rust rewrite of darktable with GTK4 through the maintained `gtk-rs` Rust bindings. Mirror darktable's GTK desktop behavior in idiomatic Rust; do not retain or call its C implementation.
- Build working import, catalog, edit, preview, save, processing, and export paths. At least one PR in every active batch must advance product behavior.
- Keep the separate `/Users/cgas/Documents/RustTable/Darktable` clone as the read-only reference. Never copy, compile, link, or retain upstream C/C++/OpenCL in RustTable.
- Port the desktop experience shift-in-place by responsibility: use Darktable's `src/gui`, `src/libs`, `src/views`, and `src/iop` as navigation for Rust GTK4 modules, preserving workflows and layout where useful without copying C APIs, source, or build machinery.
- Treat Darktable's visible GTK composition as the product reference, not as inspiration for a generic photo application. Before changing a GTK surface, inspect the matching upstream view, panel, and module sources; reproduce its information hierarchy, mode switching, panel placement, labels, and controls with GTK4 Rust widgets.
- A desktop UI PR must name the Darktable source paths it maps and include a direct visual/behavioral comparison. Prefer a faithful GTK4 equivalent over a new layout or renamed workflow unless an upstream behavior is intentionally deferred in the linked issue.
- When a capability is replaced, delete obsolete native payload from RustTable. Preserve behavior and formats, not the upstream file graph.
- A GTK4 controller owns each migrated desktop workflow. Delete superseded UI source, tests, and dependency paths in the same migration slice; never maintain two live UI implementations for one workflow.
- One backend, one UI workflow rule: a product capability has one typed service owner and one GTK4/gtk-rs controller/view path. UI modules may define only the smallest typed port needed to cross into that service; they must not grow a second backend, duplicate process/filesystem/catalog logic, or preserve an Iced compatibility surface.
- Follow the Rust crate/module structure while using `architecture/darktable-subsystems.toml` for broad upstream navigation.

## Rust rules

- Use Rust 2024 and the exact dated Rust 1.98 beta in `rust-toolchain.toml`.
- Warnings, Clippy `all`, and Clippy `pedantic` are errors. Never weaken them to land a change.
- Unsafe Rust is forbidden. If a future native boundary makes it unavoidable, require a focused issue, the smallest safe API, documented invariants, and focused tests before changing policy.
- Keep handwritten source files at or below 1,000 lines. Generated compatibility data is the only exception. Split growing code into cohesive modules before it crosses that boundary; never reduce required behavior or reject a feature merely to preserve the limit.
- Prefer the standard library, GTK4/GLib facilities, and established Rust crates over bespoke infrastructure.

## Development and tests

- Use test-driven development. Add focused deterministic coverage for every behavior change and regression.
- Prefer `CARGO_BUILD_JOBS=10` for local Rust builds and checks.
- Keep external runtimes, packaging, full reference execution, and other expensive checks out of unit tests.
- `cargo xtask check` is the complete local gate: source policy, formatting, strict Clippy, all-target/all-feature tests, operation data, fixtures, and standard dependency checks.
- Local hooks are optional convenience. Pull-request CI on Linux, macOS, Windows, and dependency checks is the merge authority.
- Extended coverage and distribution run after merge or for releases. Do not recreate validation schedulers, timing budgets, wave planners, or receipt graphs.
- Run independent hosted jobs in parallel and use caches; do not impose local elapsed-time caps.

## Issues and pull requests

- GitHub issues, labels, milestones, and priorities are the sole planning source of truth. Do not mirror, hash, compile, or rewrite issue prose in repository tooling.
- Select dependency-ready work by priority label, P0 through P4.
- A PR normally groups two directly coupled issues into one complete, shift-in-place Rust vertical slice; keep their shared upstream responsibility explicit in the issue and PR body.
- Work in batches of at most two PRs; both must merge before the next batch starts. Use multiple agents to deliver each PR, with at least one PR in every batch replacing real darktable behavior in Rust.
- Open PRs ready for review with Why, How, Validation, and issue linkage. Enable squash auto-merge after local validation and required review.
- Do not let hosted CI outages block locally validated progress, but fix actual CI configuration defects promptly.
- When fewer than ten open issues remain, start fresh milestone-scoped consults to propose concrete product issues.

## Worktrees and remotes

- Use `/Users/cgas/Documents/RustTable/worktrees` for development worktrees and `scripts/dev/create-agent-worktree.sh --issue NUMBER` to start from `origin/main`.
- Reserve `/Users/cgas/Documents/RustTable/RustTable` for repository management; it tracks the fork's `origin/main`.
- `origin` and `upstream` are `cgasgarth/RustTable`; `darktable` is fetch-only. Never push to `darktable-org/darktable`.
- Protect `main` and `master` from direct commits. Use squash-merged GitHub PRs.
- Preserve unrelated and untracked user files. Copy only explicit untracked inputs into a new worktree.

## Agent orchestration

- Reuse a completed agent only when its context and clean worktree directly continue the same PR.
- Leave long-running agents running without routine polling; completion messages wake the orchestrator. Inspect only on completion or an urgent dependency.
- Keep each agent in an isolated worktree. More agents may collaborate inside one PR, but active PR batch limits still apply.
- Keep at least two agents on disjoint, product-facing Rust implementation slices whenever a batch is active. Prefer a coherent upstream subsystem's composition, persistence, UI, render, or test slices over setup, policy, or workflow work.
- Use up to four concurrent agents when the slices are genuinely disjoint and materially accelerate the active migration batch; keep at least two product slices working, and reduce concurrency when shared-file conflicts outweigh the speedup.
- Combine tightly coupled implementation work into the active PR rather than serializing tiny scaffolding PRs; do not combine unrelated subsystems just to increase PR size.
- Re-read the current issue and parent issue before follow-up work because GitHub scope may change.

## Computer Use installation

- `bun run install:computer-use` installs exactly `~/Applications/rusttable - latest.app` with bundle ID `com.cgasgarth.rusttable.latest`.
- Rerunning replaces that app transactionally and must not create duplicates. Preserve unrelated applications.

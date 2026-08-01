# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative contract

Read `AGENTS.md` and `TASK.md` completely before migration work. They define the source baseline, completion criteria, branch policy, validation gate, and delivery loop. Read `docs/migration-workflow-lessons.md` before authoring a milestone workflow; it records concrete prior failures and prevention checks.

- Use `/Users/cgas/Documents/RustTable/RustTable` as the canonical integration checkout on the active migration branch. Any number of temporary worker worktrees/workflows may be active concurrently when each has exclusive ownership and host capacity supports it; never use worker worktrees for independent delivery branches or PRs.
- The repository default branch and pull-request base are `main`; target `main` for migration PRs and do not use the stale `master` branch as the base.
- `src/`, retained native build files, and the sibling `../Darktable` checkout are read-only porting oracles. Never modify them or compile, link, ship, or FFI-call retained C/C++/OpenCL code.
- Use Cargo for RustTable. Do not run root `build.sh` or CMake.
- Treat Rust code predating strict-reset commit `a5a039af2319275c11455888e9fb02ee0288916f` as provisional unless a later strict milestone explicitly re-inspected and validated the matching native responsibility.
- Port responsibilities directly from source. Preserve constants, formats, ordering, state transitions, failures, processing semantics, and UI composition; do not substitute plausible behavior.
- Keep one meaningful milestone PR active at a time, but keep implementation continuously queued: as a lane reaches integration or review, start the next dependency-ready source audit and exclusive leaf work without waiting idle. Complete local validation, open and squash-merge the current PR, sync `main`, then continue the queued stream without waiting for user check-ins; stop only when the user explicitly asks.
- Never claim a complete native-file port or delete retained source until every `TASK.md` completion criterion and remaining native dependency permits it.

## Development commands

Run the repository doctor before implementation:

```sh
bash scripts/dev/doctor.sh
```

Common commands:

```sh
cargo build --package rusttable-app --bin rusttable-app --locked
cargo run --package rusttable-app --bin rusttable-app --locked
cargo check --workspace --all-targets --all-features --locked
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

Use the partial gate during iteration to validate changed packages and their reverse workspace dependencies. It compares against `origin/main` by default; pass `--base REF` when another merge base is required:

```sh
cargo xtask check --changed
cargo xtask check --changed --base REF
```

Use focused checks while iterating:

```sh
cargo check --package rusttable-processing --all-targets --all-features --locked
cargo clippy --package rusttable-processing --all-targets --all-features --locked -- -D warnings
cargo test --package rusttable-processing --test colorzones --locked
cargo test --package rusttable-processing --test colorzones <test-name> --locked -- --exact --nocapture
cargo test --package rusttable-app --lib <module::test> --locked -- --exact --nocapture
```

GTK integration targets declared with `harness = false` run as whole binaries; do not pass libtest filters:

```sh
cargo test --package rusttable-ui --test colorcorrection_gtk_boundary --locked
cargo test --package rusttable-app --test darkroom_shell_runtime_smoke --locked
```

The final gate for each coherent milestone is one uninterrupted successful run:

```sh
cargo xtask check --parallel
```

Use `cargo xtask <command> --help` before invoking unfamiliar contract, fixture, shader, reference, or distribution subcommands. Let Cargo choose host parallelism.

## Worktree and build-artifact hygiene

- Cargo `target/` directories are disposable build artifacts, not migration evidence. Never preserve them merely because a source worktree is being parked.
- After a worker or workflow reports completion, verify that no live `claude`, `cargo`, `rustc`, `clippy`, `rust-analyzer`, test, or hook process references the worktree. Then remove only that inactive worktree's `target/` directory before parking it.
- Never remove, move, or clean a `.claude/worktrees/wf_*` directory while it is locked, listed as active, or referenced by a live process. Wait for the workflow completion notification and preserve dirty source worktrees until their changes are reviewed or explicitly discarded.
- Do not run `cargo clean` or remove `target/` in the canonical checkout or any active worker worktree. Rebuilding is acceptable only after a worktree is confirmed inactive.
- Park source-bearing candidates and their branches when they may be needed for review or restacking, but do not automatically delete those worktrees or branches. Clean their build artifacts instead.
- Use `git worktree list --porcelain` and a process-path check before cleanup; use `git worktree prune --dry-run` before pruning stale administrative metadata. Pruning must never be used to discard a live or source-bearing worktree.
- Treat Rust Analyzer diagnostics that reference an actively rebuilding worker target as potentially stale until that target finishes; do not delete or rebuild that target during the active workflow.

## Runtime architecture

The main dependency flow is:

1. `rusttable-core` defines typed IDs, immutable edits, operations, revisions, metadata, and configuration contracts.
2. Catalog/import/image crates decode sources and persist catalog, history, metadata, and provenance through domain/repository boundaries.
3. `rusttable-processing` validates and compiles edits into typed operation graphs. It owns descriptors, migrations, registry factories, colorspace contracts, CPU equations, masks, and output transforms.
4. `rusttable-pixelpipe` executes immutable snapshots. It owns CPU/GPU selection, canonical CPU fallback, cancellation, scheduling, ROI/tiling, caches, and publication gates.
5. `rusttable-render` performs final presentation, target resampling, output color handling, encoding, diagnostics, and thumbnail lifecycle.
6. `rusttable-ui` owns GTK presentation and controls. `rusttable-app` is the composition root that connects persistence, workers, preview/export services, edit routing, and the GLib main loop; do not duplicate numerical or business behavior there.

A processing operation is cross-cutting. Follow established strict ports such as Velvia, Vibrance, and Color Contrast through the applicable history codec/import route, typed parameters, descriptor and ordering, compiler/registry/reconstruction, CPU evaluation, snapshot identity, GPU qualification and fallback, app edit persistence, UI, generated architecture contracts, and focused tests. Adding only a numerical kernel does not route an operation into production.

## UI and end-to-end validation

Use source-derived GTK hierarchy, allocations, state, and interactions as the specification. Screenshots support validation but must not determine geometry.

Keep E2E validation conventional and project-owned:

1. Pure Rust tests cover editor state, processing, persistence, and rendering contracts.
2. Existing non-activating GTK boundary and app runtime-smoke tests cover production widget wiring, controllers, allocations, and paint behavior.
3. Existing repository scripts build, transactionally install, launch, and smoke-test the real app.
4. Use the installed CUA Computer Use MCP for focused live operation of the real app after automated checks converge. Launch through project-owned scripts, keep the target in an isolated temporary app/catalog/config state, inspect accessibility and matched-window geometry, and verify interactions in a background CUA session without stealing foreground focus.
5. Use small AppleScript/System Events checks only when a focused macOS interaction or matched-window capture cannot be exercised through project tests or CUA.

Do not build a separate automation framework or survey external UI-driving ecosystems. Live automation must be deliberate and compare RustTable with the runnable Darktable reference at matched normal-window bounds.

## Model-aware workflows

Use workflows to maintain a rolling stream of dependency-ready migration work while keeping shared-file ownership explicit. Keep up to two workflows active whenever dependency-ready work and host capacity allow, and refill an available workflow slot as soon as one completes; do not start a third concurrent workflow. One active PR constrains delivery, not implementation concurrency: keep source audits, leaf ports, focused contracts, diagnostics, and adversarial reviews running in parallel wherever dependencies permit; use the canonical checkout for integration and delivery, and fan out temporary worker worktrees within the two-workflow cap, then converge through one integration owner. Do not leave the migration idle between milestones; stop only on an explicit user request or a genuine repository/environment blocker that requires user input.

- Before implementation fan-out, produce a source-responsibility inventory covering native functions and constants, Rust callers, ownership/lifetime boundaries, behavior-preserving tests, writable file ownership, and explicit deferred responsibilities.
- Run source/caller/test research in parallel by responsibility.
- Parallelize non-overlapping leaf modules, focused tests/contracts, GPU work, UI/editor work, app/persistence, catalog/import, render/export, pixelpipe infrastructure, and compiler-diagnostic repair batches wherever dependencies permit; the workflow count is capped at two, but each workflow should use as many exclusive worker lanes as ownership and host capacity safely support.
- Keep implementation and adversarial verification context-independent: reviewers inspect source evidence and the actual worktree/diff, try to refute behavioral equivalence, and do not inherit the implementer's rationale as fact.
- Treat compiler diagnostics as a refreshed integration work queue. The orchestrator captures a fresh focused diagnostic snapshot in the canonical checkout, partitions errors by exclusive crate/file ownership, dispatches one worker per non-overlapping queue item subject only to available worker capacity, and refreshes diagnostics after each mutation batch before assigning the next queue. Never assign agents from stale diagnostics.
- Use Luna at max effort for workflow source research, independent constant/format/order verification, well-scoped implementations, focused tests, and targeted adversarial review passes. Reviewers should stay within the changed responsibility and explicit acceptance boundaries and return concise findings rather than broad audits or review panels.
- Use Sol at high effort for cross-crate problem solving, shared pixelpipe/GPU/state integration, and GTK UI work.
- Assign one writer at a time to exhaustive-match and integration hubs in the canonical checkout. Every worker worktree must have exclusive writable paths; inspect each worktree diff before integrating and reject overlapping ownership.
- Worker commands have a hard two-minute budget. Workers may run only focused checks expected to finish inside it, must stop commands that cross it, and must report remaining validation to the orchestrator instead of blocking dependent stages.
- Workers must not run workspace/package-wide gates, the full gate, or `cargo xtask check --changed` when its reverse-dependency closure is broad. The orchestrator runs long validation in the canonical checkout after worker changes converge.
- Workers may commit only scoped assigned changes; no worker may push, open, merge, or otherwise create a separate PR. The canonical checkout owns the single milestone commit/PR lifecycle.
- The orchestration agent is a manager, not the default implementer. Use workflows for source research, implementation, focused tests, integration planning, adversarial review, and validation. Keep direct canonical-checkout code changes to very small cleanups, conflict resolution, or mechanically applying an already-reviewed worker patch; route substantive implementation through an isolated workflow worker with explicit ownership.
- A workflow must report its phase ownership, exact writable paths, source evidence, focused validation, and deferred work. The manager inspects every worker diff, integrates only scoped changes, and starts the next workflow from refreshed diagnostics rather than stale assumptions.
- Agents must never modify retained native sources or the sibling Darktable checkout.

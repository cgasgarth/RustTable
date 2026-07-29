# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative contract

Read `AGENTS.md` and `TASK.md` completely before migration work. They define the source baseline, completion criteria, branch policy, validation gate, and delivery loop. Read `docs/migration-workflow-lessons.md` before authoring a milestone workflow; it records concrete prior failures and prevention checks.

- Use `/Users/cgas/Documents/RustTable/RustTable` as the canonical integration checkout on `codex/file-by-file-migration`. Up to three temporary worker worktrees may be active concurrently; never create more than three, and never use them for independent delivery branches or PRs.
- `src/`, retained native build files, and the sibling `../Darktable` checkout are read-only porting oracles. Never modify them or compile, link, ship, or FFI-call retained C/C++/OpenCL code.
- Use Cargo for RustTable. Do not run root `build.sh` or CMake.
- Treat Rust code predating strict-reset commit `a5a039af2319275c11455888e9fb02ee0288916f` as provisional unless a later strict milestone explicitly re-inspected and validated the matching native responsibility.
- Port responsibilities directly from source. Preserve constants, formats, ordering, state transitions, failures, processing semantics, and UI composition; do not substitute plausible behavior.
- Keep one meaningful milestone PR active at a time. Complete local validation, open and squash-merge it, sync `main`, then continue without waiting for user check-ins.
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

Use workflows to advance one dependency-ready milestone at a time while keeping shared-file ownership explicit. One active PR constrains delivery, not implementation concurrency: use the canonical checkout for integration and delivery, up to three temporary worker worktrees for independent batches, then converge through one integration owner.

- Before implementation fan-out, produce a source-responsibility inventory covering native functions and constants, Rust callers, ownership/lifetime boundaries, behavior-preserving tests, writable file ownership, and explicit deferred responsibilities.
- Run source/caller/test research in parallel by responsibility.
- Parallelize non-overlapping leaf modules, focused tests/contracts, GPU work, UI/editor work, and compiler-diagnostic repair batches where dependencies permit; use the three-worktree cap rather than serializing independent work.
- Keep implementation and adversarial verification context-independent: reviewers inspect source evidence and the actual worktree/diff, try to refute behavioral equivalence, and do not inherit the implementer's rationale as fact.
- Treat compiler diagnostics as a refreshed integration work queue. The orchestrator captures a fresh focused diagnostic snapshot in the canonical checkout, partitions errors by exclusive crate/file ownership, dispatches at most three worker worktrees, and refreshes diagnostics after each mutation batch before assigning the next queue. Never assign agents from stale diagnostics.
- Use Luna at medium effort for source research and independent constant/format/order verification.
- Use Luna at xhigh effort for well-scoped, mechanical implementations and focused tests.
- Use Luna at medium effort for targeted adversarial review passes. Review only the changed responsibility and explicit acceptance boundaries; return concise findings rather than broad audits or review panels.
- Use Sol at high effort for cross-crate problem solving, shared pixelpipe/GPU/state integration, and GTK UI work.
- Assign one writer at a time to exhaustive-match and integration hubs in the canonical checkout. Every worker worktree must have exclusive writable paths; inspect each worktree diff before integrating and reject overlapping ownership.
- Worker commands have a hard two-minute budget. Workers may run only focused checks expected to finish inside it, must stop commands that cross it, and must report remaining validation to the orchestrator instead of blocking dependent stages.
- Workers must not run workspace/package-wide gates, the full gate, or `cargo xtask check --changed` when its reverse-dependency closure is broad. The orchestrator runs long validation in the canonical checkout after worker changes converge.
- Workers may commit only scoped assigned changes; no worker may push, open, merge, or otherwise create a separate PR. The canonical checkout owns the single milestone commit/PR lifecycle.
- Agents must never modify retained native sources or the sibling Darktable checkout.

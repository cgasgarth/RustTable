# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative contract

Read `AGENTS.md` and `TASK.md` completely before migration work. They define the source baseline, completion criteria, branch policy, validation gate, and delivery loop. Read `docs/migration-workflow-lessons.md` before authoring a milestone workflow; it records concrete prior failures and prevention checks.

- Work only in this checkout on `codex/file-by-file-migration`; do not create worktrees.
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

Without Computer Use, keep E2E validation conventional and project-owned:

1. Pure Rust tests cover editor state, processing, persistence, and rendering contracts.
2. Existing non-activating GTK boundary and app runtime-smoke tests cover production widget wiring, controllers, allocations, and paint behavior.
3. Existing repository scripts build, transactionally install, launch, and smoke-test the real app.
4. Use small AppleScript/System Events checks only for focused macOS interactions or matched-window captures that the project tests cannot exercise.

Do not build a separate automation framework or survey external UI-driving ecosystems. Foreground automation must be deliberate, use isolated temporary app/catalog/config state, and compare RustTable with the runnable Darktable reference at matched normal-window bounds.

## Model-aware workflows

Use workflows to advance one dependency-ready milestone at a time while keeping shared-file ownership explicit. One active PR constrains delivery, not implementation concurrency: default to a broad parallel fan-out for independent work, then converge through one integration owner.

- Run source/caller/test research in parallel by responsibility.
- Parallelize non-overlapping leaf modules, focused tests/contracts, GPU work, and UI/editor work where dependencies permit; use roughly 4–8 active agents for a substantial milestone rather than serializing independent work.
- Use Luna at medium effort for source research and independent constant/format/order verification.
- Use Luna at xhigh effort for well-scoped, mechanical implementations and focused tests.
- Use Luna at medium effort for targeted adversarial review passes. Review only the changed responsibility and explicit acceptance boundaries; return concise findings rather than broad audits or review panels.
- Use Sol at high effort for cross-crate problem solving, shared pixelpipe/GPU/state integration, and GTK UI work.
- Assign one writer at a time to exhaustive-match and integration hubs. Parallelize read-only verification and non-overlapping files, then run a separate integration/review pass.
- Agents must never modify retained native sources or the sibling Darktable checkout.

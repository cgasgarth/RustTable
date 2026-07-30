# RustTable Engineering Guidelines

## Migration contract

- RustTable is a complete Rust rewrite of Darktable. Production code uses Rust 2024 and GTK4 through `gtk-rs`; it must never compile, link, or call the retained C/C++/OpenCL implementation.
- The authoritative migration baseline is Darktable commit `d8628e8103989bc4ef06dbfb9fd01f3809f884bf`, the parent of RustTable's bulk native-source deletion. Its original `src/`, `data/`, `po/`, `doc/`, `dev-doc/`, `packaging/`, `tools/`, and build files remain in this repository as a read-only porting oracle.
- Keep the current Cargo workspace and crate boundaries. Within the appropriate crate, mirror the original path and responsibility so a reviewer can immediately find both sides of a port. For example, `src/dtgtk/sidepanel.c` maps to `crates/rusttable-ui/src/dtgtk/sidepanel.rs`.
- Work file by file. Before implementing a Rust replacement, read the complete source file, its directly coupled local headers/helpers, and relevant upstream tests. Preserve constants, state transitions, ordering, formats, error behavior, and UI composition; adapt only what Rust ownership, safety, established crates, or GTK4 APIs require.
- Do not invent substitute workflows, simplified processing semantics, arbitrary geometry, or parallel abstractions. An unported capability must remain explicitly unavailable rather than presenting plausible but incorrect behavior.
- Treat all existing Rust application behavior as provisional. Retain it only when tests and source comparison prove that it faithfully ports a specific Darktable responsibility; otherwise replace or remove it.
- Never edit retained baseline files. A port deletes the replaced C/H/OpenCL source only after the Rust path is used by the application, source-derived tests pass, parity is reviewed, and no remaining retained source depends on it. Keep original assets and translations until their Rust consumers are complete.
- The separate `/Users/cgas/Documents/RustTable/Darktable` checkout remains the runnable visual/behavioral reference. It is not a Git remote or contribution target for RustTable.

## Rust rules

- Use the pinned Rust 1.98 beta, Rust 2024, strict warnings, Clippy `all`/`pedantic`, and `unsafe_code = "forbid"`.
- Unsafe Rust requires an explicit, focused review proving that no safe implementation is practical, with documented invariants and boundary tests. Do not relax workspace policy preemptively.
- Do not impose a line-count limit on ports. Keep a Darktable file's responsibility together when that makes source comparison clearer; split into nested responsibility-based modules only when the original structure or Rust maintainability genuinely supports it.
- Put a source-lineage module comment on every direct port naming the baseline source file(s). Keep size-driven children nested under that mapped parent.
- Prefer the standard library, GTK4/GLib facilities, and established Rust crates over bespoke infrastructure, while preserving Darktable's observable behavior and formats.

## Development and validation

- Use test-driven development. Derive regression cases, boundary values, ordering, and failure behavior from the matching Darktable source and tests.
- `cargo xtask check` is the complete local commit gate: formatting, strict Clippy, all-target/all-feature tests, rustdoc, fixtures, and relevant repository policy.
- Let Cargo detect host parallelism. Do not pass `CARGO_BUILD_JOBS` or test-thread counts on individual commands.
- The retained baseline is not part of the Cargo build. Keep it unchanged until verified ports allow individual source files to be deleted; use normal Git review instead of bespoke policy machinery.
- Keep precommit output concise on success and actionable on failure. PR-triggered GitHub Actions remain disabled; post-merge validation may run extended platform, packaging, coverage, and distribution checks.

## Visual parity

- Launch the installed RustTable app and original Darktable app directly with Computer Use.
- Compare normal macOS windows maximized to identical working-area bounds. Do not use macOS full-screen mode, unequal window sizes, or normalized approximations.
- Use the same image, view, layout mode, selection, panel visibility, panel widths, and expanded modules.
- Derive layout from Darktable source and live GTK allocations, never screenshot pixel guesses. Geometry, colors, typography, controls, rail behavior, and interactions are acceptance criteria.
- Inspect lighttable, darkroom, top/bottom chrome, both rails, histogram, filmstrip/timeline, implemented modules, collapsed/expanded states, resizing, and edited-preview propagation before accepting a UI milestone.

## Git and delivery

- Use `/Users/cgas/Documents/RustTable/RustTable` as the canonical integration checkout for migration development. Any number of temporary Git worktrees may be active concurrently when batches have exclusive ownership and host capacity supports them; clean completed worktrees before broad repository gates.
- Work on the long-lived `codex/file-by-file-migration` branch. Keep `main` protected from direct commits.
- Commit coherent source-file ports as they complete. Open a ready-for-review PR only for a meaningful migration milestone, then squash-merge it.
- GitHub issues track major milestones and concrete defects discovered during faithful ports. Do not create abstraction-first backlogs, one issue per trivial step, artificial PR batches, or priority-driven work that skips source order.
- A milestone PR must explain the Darktable files replaced, the Rust destinations, deleted baseline files, behavior retained, known unported dependencies, and validation evidence.
- `origin` is `cgasgarth/RustTable`. Do not add or fetch Darktable as an automatic RustTable remote; use the separate local checkout and pinned in-repository baseline.

## Agent orchestration

- The canonical integration checkout owns `codex/file-by-file-migration`, delivery, and the single active milestone PR. Use as many temporary Git worktrees/workflows as independent implementation, diagnostics, tests, review, and subsystem lanes require, bounded by available host capacity rather than a fixed count.
- Each worker worktree has exclusive file ownership. Never assign the same writable file to two worktrees; keep exhaustive-match registries, pixelpipe dispatch, app composition, and other shared hubs with one integration owner in the canonical checkout.
- Workers may analyze, test, review, edit, and commit only their assigned paths. The orchestrator inspects and integrates those scoped changes, runs final validation in the canonical checkout, and owns all push and PR actions.
- Give agents exact Darktable and Rust paths. Require findings about implemented-but-incorrect behavior; do not report functionality that is merely unported.
- Prefer parallel migration work that advances faithful Rust implementation over process, PR, or orchestration churn. Do not poll running agents routinely; completion notifications wake the orchestrator.

## Computer Use installation

- `bun run install:computer-use` installs exactly `~/Applications/rusttable - latest.app` with bundle ID `com.cgasgarth.rusttable.latest`.
- Rerunning replaces that app transactionally and must not create duplicates. Preserve unrelated applications.

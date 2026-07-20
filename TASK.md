# RustTable Task

## Goal

Rewrite darktable completely in Rust with GTK4 through `gtk-rs`, preserving useful image-processing behavior, catalog and metadata formats, editing workflows, compatibility history, and reference-render fidelity. All application UI code is Rust; the separate darktable C checkout is behavior/layout reference only.

## Source of truth

- GitHub milestones define delivery areas.
- GitHub issues, labels, priorities, dependencies, and state define executable work.
- Do not commit issue snapshots, hashes, generated queues, readiness calculations, or source-file ownership databases.
- One coherent issue maps to one ready-for-review, squash-merged PR.

## Engineering constraints

- Use the pinned Rust 1.98 beta, Rust 2024, strict warnings/Clippy, and `unsafe_code = "forbid"`.
- Keep handwritten source files at or below 1,000 lines.
- Prefer established crates and language/framework features.
- Use deterministic test-driven development.
- Keep darktable C/C++/OpenCL only in the separate reference clone.

## Product evidence

- Preserve the pinned darktable identity.
- Preserve the generated operation/history compatibility manifest.
- Preserve the real fixture corpus, differential reference runner, product benchmarks, and distribution tooling.
- Use the short subsystem map for navigation; do not recreate exhaustive file-to-issue accounting.

## Validation

- Local: `cargo xtask check`.
- Pull requests: Linux formatting/Clippy/tests/product-data checks, macOS and Windows checks/tests, and dependency/security checks.
- Post-merge/release: coverage and distribution.
- Local hooks are optional and uncapped. Pull-request CI is authoritative.

## Execution loop

1. Select dependency-ready work by priority.
2. Create an isolated worktree from `origin/main`.
3. Write the focused failing test or executable acceptance check.
4. Implement the smallest complete product slice.
5. Run `cargo xtask check`.
6. Open one ready PR with Why, How, Validation, and issue linkage.
7. Enable squash auto-merge and integrate before exceeding the active two-PR batch.

# RustTable Task

## Goal

Rewrite Darktable completely in safe Rust with GTK4/`gtk-rs`, preserving its observable editing, catalog, metadata, image-processing, export, packaging, and desktop behavior. Complete the migration file by file from the pinned Darktable baseline while retaining the existing Rust workspace structure.

## Baseline

- Pinned source commit: `d8628e8103989bc4ef06dbfb9fd01f3809f884bf`.
- Retain the original source, assets, translations, documentation, and build metadata in their original paths as a read-only oracle.
- Keep Rust production code under the existing `crates/` workspace with nested paths that make the original source mapping obvious.
- Do not compile or link retained C/C++/OpenCL into the Rust application.

## File-port completion

A source file is ported only when:

1. Its complete behavior and directly coupled declarations were inspected.
2. The Rust parent module names the original source path.
3. Source-derived tests cover important behavior, boundaries, ordering, and failures.
4. The production Cargo application uses the Rust implementation.
5. UI or rendering behavior is compared against the runnable original when applicable.
6. Incorrect provisional Rust behavior is removed rather than kept as a compatibility layer.
7. The replaced baseline file can be deleted without breaking still-retained source dependencies.

Large source files may remain large when that preserves a clear mechanical comparison. Split them into nested responsibility-based children only when the original design or a natural Rust boundary supports the split, never to satisfy an arbitrary size target.

## Validation

- Complete local gate: `cargo xtask check`.
- UI gate: equal-size normal-window Computer Use comparison against original Darktable.
- Milestone gate: full local validation plus relevant platform, packaging, fixture, and differential checks.
- Unsafe Rust remains forbidden unless separately justified and reviewed.

## Execution loop

1. Select the next dependency-ready Darktable source file in source order.
2. Read that file, coupled headers/helpers, tests, and callers completely.
3. Write failing Rust tests from the source contract.
4. Port the complete responsibility into the matching nested crate path.
5. Replace incorrect provisional Rust code and route production callers to the port.
6. Run focused tests, then `cargo xtask check`.
7. Delete the original file only when its completion criteria are proven.
8. Commit the coherent file port on `codex/file-by-file-migration`.
9. Open and squash-merge a PR only at a meaningful migration milestone.

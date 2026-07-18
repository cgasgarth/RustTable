# RustTable Engineering Guidelines

## Project direction

- RustTable is a complete rewrite of darktable in Rust; do not incrementally port the existing C implementation.
- After a darktable capability is ported, delete its corresponding legacy C/C++ implementation and obsolete CMake/native build, packaging, and generated-native files from RustTable. Keep historical C/C++ only in the separate local darktable reference clone at `/Users/cgas/Documents/RustTable/upstream`; do not retain a fallback or parallel native implementation in this repository.
- Use `iced` for the user interface.
- Prefer established Rust crates where they materially improve correctness or maintainability. Prefer the Rust standard library and framework facilities when they are sufficient.
- Keep the architecture modular, testable, and suitable for independently replaceable components.

## Rust safety and compiler strictness

- Rust code must compile with strict diagnostics and all warnings treated as errors.
- Enable and maintain strict Clippy and rustfmt checks in CI and local hooks.
- Unsafe Rust is forbidden by default. Use it only when it is absolutely necessary, isolate it behind the smallest safe API, document the safety invariants at the unsafe boundary, and add focused tests.
- Do not weaken lints to make code pass. Any lint exception must be narrow, justified in a comment, and reviewed.
- Keep dependency versions bounded and review new dependencies for maintenance, license, security, and build-time cost.

## File size

- Keep every hand-written source file at or below 1,000 lines.
- Generated files are the only exception. Mark generated files clearly and do not hand-edit them.
- Split modules before they approach the limit; do not use the limit as a reason to create opaque abstractions.

## Test-driven development

- Use strong test-driven development: write a focused failing test first, implement the smallest correct change, then refactor.
- Every behavior change needs appropriate unit, integration, property, snapshot, or end-to-end coverage.
- Keep tests deterministic, isolated, and fast. Avoid sleeps and network access in unit or PR checks.
- Defects require a regression test unless the change is strictly non-functional.

## Shift-left validation

- Pre-commit checks must run independent checks in parallel where practical and complete within 30 seconds.
- Pre-push checks must complete within 1 minute.
- Pull-request GitHub Actions must complete within 2.5 minutes. Keep PR workflows focused on fast, high-signal checks; move heavyweight or exhaustive validation to a merge-to-main workflow invoked after integration.
- Formatting, linting, compilation, tests, dependency checks, file-size checks, and unsafe-code checks should fail as early as practical.
- Measure hook and workflow duration when changing validation so the time budgets remain enforceable.

## Worktrees and Git remotes

- Use `/Users/cgas/Documents/RustTable/worktrees` for all development worktrees.
- Do not develop directly in the `fork` checkout; reserve it for repository management and worktree creation.
- In the RustTable checkout, `origin` and `upstream` refer to `https://github.com/cgasgarth/RustTable.git`; the original project is retained as the read-only `darktable` remote for history and reference.
- Never push to or open pull requests against `darktable-org/darktable`.
- Use focused branches and descriptive commit messages. Keep changes small enough to review and validate quickly.
- Open every pull request ready for review by default. Do not open draft pull requests unless the user explicitly requests a draft; if tooling creates a draft, mark it ready before handoff.

For workflow/orchestration follow-up work, reuse a completed worker only when its prior context and isolated worktree are clean, relevant, and materially continue the new issue; otherwise start a fresh worker. Close completed workers before reuse, preserve the active two-worker cap unless explicitly relaxed, keep worktrees isolated, and maintain one GitHub issue per PR.

## Documentation and review

- Document architectural decisions, public APIs, safety invariants, and non-obvious performance tradeoffs.
- Keep code, tests, and documentation consistent with the current RustTable design; do not copy stale darktable assumptions into new APIs without validation.
- Before submitting changes, inspect the diff, run the fastest relevant checks, and report any skipped validation explicitly.

## Issue queue and consults

- Treat open GitHub issues and milestones as the migration plan's source of truth; each issue maps to exactly one pull request.
- When fewer than 10 open GitHub issues remain, kick off fresh consults to scope additional work against the current repository and milestones.
- Prompt each consult for concrete issue-sized proposals with a title, rationale, scope, acceptance criteria, dependencies, and recommended milestone.
- Convert accepted consult proposals into GitHub issues before implementation, then work from those issues without using consult chats as a second task tracker.

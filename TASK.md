# RustTable Task

## Goal

Rewrite darktable completely in Rust, using iced for the UI, while preserving the useful domain behavior and history context of the original project.

## Source of truth

- GitHub milestones define broad delivery areas.
- GitHub issues define the executable work queue.
- Each issue maps to exactly one pull request.
- Every pull request targets `main` and uses squash merge.
- Do not commit directly to `main` or `master`.
- Use `/Users/cgas/Documents/RustTable/worktrees` for development worktrees.

## Engineering constraints

- Use strict Rust compiler diagnostics, rustfmt, and Clippy; warnings are errors.
- Avoid unsafe Rust. Permit it only when absolutely necessary, isolate it, document safety invariants, and test it.
- Keep hand-written source files at or below 1,000 lines; generated files are the only exception.
- Prefer established Rust crates and standard-library/framework features where appropriate.
- Use test-driven development with deterministic, focused regression coverage.

## Validation policy

- Pre-commit: complete local merge-readiness checks; no elapsed-time cap.
- Pre-push: complete local merge-readiness checks; no elapsed-time cap.
- Pull-request GitHub Actions: none.
- Pushes to `main`: rerun the complete gate and all exhaustive or heavyweight validation.

## Execution loop

1. Choose one open GitHub issue and its milestone.
2. Create one focused worktree branch for that issue.
3. Write the failing test or executable acceptance check first.
4. Implement the smallest complete change and validate it within the applicable budget.
5. Open one PR that closes only that issue, then squash-merge it.
6. Start the next issue only after the prior issue is integrated or explicitly blocked.

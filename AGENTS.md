# RustTable Engineering Guidelines

## Project direction

- RustTable is a complete rewrite of darktable in Rust; do not incrementally port the existing C implementation.
- After a darktable capability is ported, delete its corresponding legacy C/C++ implementation and obsolete CMake/native build, packaging, and generated-native files from RustTable. Keep historical C/C++ only in the separate local darktable reference clone at `/Users/cgas/Documents/RustTable/upstream`; do not retain a fallback or parallel native implementation in this repository.
- Use `iced` for the user interface.
- Prefer established Rust crates where they materially improve correctness or maintainability. Prefer the Rust standard library and framework facilities when they are sufficient.
- Keep the architecture modular, testable, and suitable for independently replaceable components.

## Rust safety and compiler strictness

- Rust code must compile with strict diagnostics and all warnings treated as errors.
- GitHub issue #456 is the authoritative compiler/dependency-baseline change: keep every
  compiler, dependency, cache, packaging, and validation surface derived from the
  repository's exact date-pinned Rust 1.98 beta baseline.
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

- Pre-commit is intentionally uncapped and runs the complete local Rust build, all-target/all-feature warnings-denied Clippy, and all-target/all-feature test gate alongside deterministic repository, source/native, layout, and workflow-policy checks. Pre-push has a hard 150-second budget and pull-request GitHub Actions have a hard 150-second budget.
- Schedule independent checks in parallel, but serialize checks that contend for the shared Cargo target directory; never skip or weaken pre-commit coverage to satisfy a duration target.
- Pre-push keeps formatting, policy, workspace, library test, and library lint checks local. Release-mode and other merge-only production validation remains on main.
- Hooks must clean up the complete child-process tree on success, failure, interrupt, and timeout; failures report bounded actionable excerpts and measured duration.
- Hooks must not use the network, mutate GitHub, require secrets, or run heavyweight packaging, corpus, benchmark, GUI, or merge-only validation.
- PR validation stays technical and build/test/workflow focused; issue linkage and pull-request body conventions are human/process guidance only and are never blocking GitHub Actions gates.
- Main validation retains exhaustive all-target/all-feature Rust coverage and heavyweight checks outside the local hook tiers.
- Formatting, linting, compilation, tests, dependency checks, file-size checks, and unsafe-code checks should fail as early as practical.
- Measure hook and workflow duration when changing validation so the time budgets remain enforceable.

## Worktrees and Git remotes

- Use `/Users/cgas/Documents/RustTable/worktrees` for all development worktrees.
- Do not develop directly in the `fork` checkout; reserve it for repository management and worktree creation.
- Create agent worktrees with `scripts/dev/create-agent-worktree.sh --issue NUMBER`. The script fetches `origin/main`, creates the isolated branch under the canonical `worktrees/` directory, and accepts repeated `--include PATH` options only for intentional repository-relative untracked inputs. Never copy an entire source checkout or silently copy tracked files; update the script's focused test when its safety contract changes.
- In the RustTable checkout, `origin` and `upstream` refer to `https://github.com/cgasgarth/RustTable.git`; the original project is retained as the read-only `darktable` remote for history and reference.
- Never push to or open pull requests against `darktable-org/darktable`.
- Use focused branches and descriptive commit messages. Keep changes small enough to review and validate quickly.
- Open every pull request ready for review by default. Do not open draft pull requests unless the user explicitly requests a draft; if tooling creates a draft, mark it ready before handoff.
- Keep the required pull-request sections and issue linkage in human review guidance, not in blocking GitHub Actions checks.
- After required checks pass and required review is present, enable GitHub auto-merge with squash for the pull request (`gh pr merge --auto --squash` or the equivalent UI). Do not enable auto-merge for drafts, failing checks, unresolved conflicts, or unapproved pull requests.
- Treat pre-commit as the strongest shift-left gate: it is uncapped, includes the complete local build/lint/test suite, and must clean up its owned process tree on interruption or failure. Keep pre-push at or below 150 seconds. Schedule independent checks in parallel, but serialize checks that contend for shared resources or give them isolated resources.

For workflow/orchestration follow-up work, reuse a completed worker only when its prior context and isolated worktree are clean, relevant, and materially continue the new issue; otherwise start a fresh worker. Close completed workers before reuse, keep worktrees isolated, and maintain one GitHub issue per PR.

Drain the currently open PR queue before opening a new batch. After the queue is empty, select up to two dependency-ready issues in priority order and create one ready-for-review PR per issue. Multiple workers may collaborate within each PR, but do not start or modify a third issue or PR until both PRs in the active batch merge. Treat each active issue as one atomic change batch: every file in the batch must serve that issue, and the orchestrator must validate the integrated batch before committing. Never combine unrelated issue batches in one commit or pull request. Preserve one issue per ready-for-review PR, squash auto-merge, strict checks, and no branch-protection bypass.

Leave long-running subagents running without routine polling; completion messages should wake/notify the orchestrator; inspect a worker only when its result arrives or an urgent dependency requires it; bounded waits are reserved for urgent dependencies.

## Documentation and review

- Document architectural decisions, public APIs, safety invariants, and non-obvious performance tradeoffs.
- Keep code, tests, and documentation consistent with the current RustTable design; do not copy stale darktable assumptions into new APIs without validation.
- Before submitting changes, inspect the diff, run the fastest relevant checks, and report any skipped validation explicitly.

### Computer Use installation

- `bun run install:computer-use` must install exactly the canonical `rusttable - latest` app at `~/Applications/rusttable - latest.app` with the stable `com.cgasgarth.rusttable.latest` bundle identity.
- Rerunning the installer replaces that canonical app transactionally; never create a versioned or alternate canonical path.
- Cleanup may target only repository-owned RustTable bundles and LaunchServices registrations from the repository worktrees or the known legacy `~/Applications/RustTable.app` path. Preserve unrelated applications and prefer recoverable cleanup (such as moving duplicates to the user Trash) where possible.

## Issue queue and consults

- Treat open GitHub issues and milestones as the migration plan's source of truth; each issue maps to exactly one pull request.
- When fewer than 10 open GitHub issues remain, kick off fresh consults to scope additional work against the current repository and milestones.
- Prompt each consult for concrete issue-sized proposals with a title, rationale, scope, acceptance criteria, dependencies, and recommended milestone.
- Convert accepted consult proposals into GitHub issues before implementation, then work from those issues without using consult chats as a second task tracker.

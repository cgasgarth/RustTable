# Contributing to RustTable

RustTable is a safe Rust 2024 and GTK4/gtk-rs rewrite of Darktable. The pinned native implementation remains unchanged in this repository as a non-built porting oracle, and the separate Darktable checkout is the runnable reference.

## Prepare the single checkout

Use `/Users/cgas/Documents/RustTable/RustTable` and create or switch to the long-lived `codex/file-by-file-migration` branch before implementation. Do not create Git worktrees.

```sh
git config core.hooksPath .githooks
bash scripts/dev/doctor.sh
```

## Port one complete responsibility

- Follow `TASK.md` and `AGENTS.md`; select the next dependency-ready Darktable file in source order.
- Read the complete source, coupled declarations, callers, tests, constants, assets, CSS, and GTK construction before implementing.
- Write source-derived tests, port the full responsibility into the matching nested Rust path, and route production callers to it.
- Keep unsafe code forbidden and preserve observable behavior with Rust, GTK4/GLib, and established crates.
- Do not edit, compile, link, FFI-call, or ship the retained native oracle.
- Delete a retained source file only after its Rust replacement is complete, verified, used in production, and no retained dependency still needs it.

## Validate and submit

Run the complete local commit gate:

```sh
cargo xtask check
```

The commit gate must not activate, raise, or switch desktop applications. Foreground visual review
and real Command-Q validation are separate, explicit workflows:
`bun run screenshot:ui-review -- --allow-foreground` and
`bun run smoke:macos-computer-use -- --allow-foreground`.

Commit coherent file ports directly on `codex/file-by-file-migration`. Open a ready-for-review PR only for a meaningful migration milestone; explain why and how, list exact source-to-Rust mappings, record validation and known unported dependencies, then squash merge.

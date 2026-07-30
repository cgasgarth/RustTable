set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# Format the workspace in place with the pinned toolchain.
fmt:
    cargo fmt --all

# Run the complete local merge-readiness gate.
check:
    cargo xtask check

# Run the workspace test suite with the same locked scope as xtask check.
test:
    cargo test --workspace --all-targets --all-features --locked

# Run the full pre-commit contract, including its owned build-job setting.
ci:
    bash scripts/dev/precommit-check.sh

# Launch the RustTable application.
run:
    cargo run --package rusttable-app --bin rusttable-app --locked

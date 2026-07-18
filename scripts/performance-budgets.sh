#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "$script_directory/.." && pwd)"
performance_directory="$repository_root/target/performance"
source "$repository_root/scripts/with-validation-budget.sh"

rm -rf "$performance_directory"
mkdir -p "$performance_directory"
run_with_budget 300 performance-budgets bash -c 'set -o pipefail; cargo bench -p rusttable-processing --bench performance_budgets --locked -- --check 2>&1 | tee "$1"' -- "$performance_directory/results.log"

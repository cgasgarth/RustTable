#!/usr/bin/env bash
set -euo pipefail

status=0
while IFS= read -r file; do
  # The handwritten-source cap applies to production code; test modules are
  # checked separately by the language test runner and may live beside it.
  line_count="$(awk '/^#[[]cfg\(test\)[]]/{exit} {count++} END{print count + 0}' "$file")"
  if (( line_count > 1000 )); then
    printf '%s: %s lines exceeds the 1000-line handwritten source limit\n' "$file" "$line_count" >&2
    status=1
  fi
  if rg -n '\bunsafe[[:space:]]*(\{|fn[[:space:]]|trait[[:space:]]|impl[[:space:]]|extern[[:space:]])' "$file" >/dev/null; then
    printf '%s: unsafe Rust is not allowed without an explicitly reviewed exception\n' "$file" >&2
    status=1
  fi
done < <(rg --files -g '*.rs' -g '!target/**')

exit "$status"

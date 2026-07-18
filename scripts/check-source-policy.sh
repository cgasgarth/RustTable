#!/usr/bin/env bash
set -euo pipefail

status=0
while IFS= read -r file; do
  line_count="$(wc -l < "$file")"
  if (( line_count > 1000 )); then
    printf '%s: %s lines exceeds the 1000-line handwritten source limit\n' "$file" "$line_count" >&2
    status=1
  fi
  if rg -n '\bunsafe\b' "$file" >/dev/null; then
    printf '%s: unsafe Rust is not allowed without an explicitly reviewed exception\n' "$file" >&2
    status=1
  fi
done < <(rg --files -g '*.rs' -g '!target/**')

exit "$status"


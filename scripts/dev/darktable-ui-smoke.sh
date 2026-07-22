#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

printf '%s\n' '== Darktable UI contract smoke =='
cargo test -p rusttable-ui --test darkroom_lighttable_contract --quiet
cargo test -p rusttable-ui --test ui_scale_contract --quiet

printf '%s\n' '== GTK scale source bounds =='
while IFS= read -r file; do
  lines="$(wc -l < "$file")"
  if (( lines > 1000 )); then
    printf 'FAIL %s has %s lines (limit 1000)\n' "$file" "$lines" >&2
    exit 1
  fi
done <<'FILES'
crates/rusttable-ui/src/gui/darktable_spec.rs
crates/rusttable-ui/src/gui/darktable_spec/scale.rs
crates/rusttable-ui/src/gui/darktable_components.rs
crates/rusttable-ui/src/gui/runtime/layout.rs
crates/rusttable-ui/src/gui/runtime/lighttable.rs
crates/rusttable-ui/src/views/lighttable/mod.rs
FILES

printf '%s\n' '== RAW import → thumbnail → darkroom smoke =='
cargo test -p rusttable-app --test raw_import_darkroom_smoke --quiet

printf '%s\n' 'PASS Darktable UI smoke (tokens, responsive rails/cards, synchronized resize, RAW preview path)'

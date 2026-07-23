#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

printf '%s\n' '== Darktable UI contract smoke =='
cargo test -p rusttable-ui --test darkroom_lighttable_contract --quiet
cargo test -p rusttable-ui --test ui_scale_contract --quiet

printf '%s\n' '== RAW import → thumbnail → darkroom smoke =='
cargo test -p rusttable-app --test raw_import_darkroom_smoke --quiet

printf '%s\n' 'PASS Darktable UI smoke (tokens, responsive rails/cards, synchronized resize, RAW preview path)'

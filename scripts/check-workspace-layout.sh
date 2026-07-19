#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
bun "$root/scripts/platform-support.ts" --json >/dev/null
exec bun "$root/scripts/workspace-layout.ts"

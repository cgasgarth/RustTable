#!/usr/bin/env bash
set -euo pipefail

exec cargo xtask ci main --skip-group coverage --skip-group offline-source-closure

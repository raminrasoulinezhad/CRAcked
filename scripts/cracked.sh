#!/usr/bin/env bash
# Copyright (c) 2026 Seyedramin Rasoulinezhad
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

# Launch CRAcked from a desktop icon / app-grid entry, always running the
# LATEST code in this repo.
#
# A double-clicked .desktop launcher inherits none of your shell environment,
# so this wrapper:
#   1. makes cargo available (sources ~/.cargo/env),
#   2. does an incremental `cargo build` to pick up any changes you've made
#      while developing (fast no-op when nothing changed),
#   3. launches the freshly built binary. If the build fails, it still launches
#      the last good binary so the icon always works.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck disable=SC1090
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"

BIN="$PROJECT_ROOT/src-tauri/target/debug/cracked"
LOG_DIR="$PROJECT_ROOT/.cache"
mkdir -p "$LOG_DIR"

# Rebuild to reflect in-progress development; keep going if it fails.
if command -v cargo >/dev/null 2>&1; then
    ( cd "$PROJECT_ROOT/src-tauri" && cargo build ) >"$LOG_DIR/build.log" 2>&1 || \
        echo "Build failed — launching last good binary. See $LOG_DIR/build.log" >&2
fi

if [[ ! -x "$BIN" ]]; then
    echo "No CRAcked binary found at $BIN and build failed." >&2
    echo "Run: cd '$PROJECT_ROOT/src-tauri' && cargo build" >&2
    exit 1
fi

exec "$BIN" "$@"

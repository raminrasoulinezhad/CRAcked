#!/usr/bin/env bash
# Copyright (c) 2026 Seyedramin Rasoulinezhad
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

# Install (or remove) the CRAcked desktop launcher on Linux.
#
#   ./scripts/install-desktop.sh            # install: app grid + desktop icon
#   ./scripts/install-desktop.sh --uninstall
#
# The launcher runs the app straight from this repo (rebuilding on each launch),
# so it always reflects your in-progress development. Idempotent: re-running
# re-points the launchers at the current repo location.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"
TARGET_NAME="cracked.desktop"

uninstall() {
    rm -f "$APPS_DIR/$TARGET_NAME" "$DESKTOP_DIR/$TARGET_NAME"
    command -v update-desktop-database >/dev/null && update-desktop-database "$APPS_DIR" 2>/dev/null || true
    echo "Removed CRAcked launchers."
}

if [[ "${1:-}" == "--uninstall" ]]; then
    uninstall
    exit 0
fi

# 1. Make the launcher executable.
chmod +x "$PROJECT_ROOT/scripts/cracked.sh"

# 2. Render the .desktop with this repo's absolute path baked in.
rendered="$(sed "s#__PROJECT_ROOT__#$PROJECT_ROOT#g" "$PROJECT_ROOT/packaging/$TARGET_NAME")"

# 3a. App-grid / search entry.
mkdir -p "$APPS_DIR"
printf '%s\n' "$rendered" > "$APPS_DIR/$TARGET_NAME"
chmod +x "$APPS_DIR/$TARGET_NAME"

# 3b. Desktop double-click icon (marked trusted so GNOME launches it).
if [[ -d "$DESKTOP_DIR" ]]; then
    printf '%s\n' "$rendered" > "$DESKTOP_DIR/$TARGET_NAME"
    chmod +x "$DESKTOP_DIR/$TARGET_NAME"
    gio set "$DESKTOP_DIR/$TARGET_NAME" "metadata::trusted" true 2>/dev/null || true
fi

# 4. Refresh the desktop database so it shows up immediately.
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS_DIR" 2>/dev/null || true

echo "Installed CRAcked:"
echo "  app grid : $APPS_DIR/$TARGET_NAME"
[[ -d "$DESKTOP_DIR" ]] && echo "  desktop  : $DESKTOP_DIR/$TARGET_NAME"
echo
echo "Press Super and type 'CRAcked', or double-click the desktop icon."
echo "On the desktop icon's first run you may need: right-click -> Allow Launching."

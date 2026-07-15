#!/usr/bin/env bash
# Copyright (c) 2026 Seyedramin Rasoulinezhad
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

# Download the rclone binary and place it where Tauri expects a sidecar:
#   src-tauri/binaries/rclone-<target-triple>[.exe]
#
# rclone is bundled INTO the installer so Google Drive backup works out of the
# box on a friend's machine (they only do a one-time Google login). This script
# runs locally before `cargo tauri build`, and in CI before each OS build.
#
# Usage:
#   ./scripts/fetch-rclone.sh                # auto-detect this machine's triple
#   ./scripts/fetch-rclone.sh <rust-triple>  # e.g. x86_64-pc-windows-msvc
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BIN_DIR="$PROJECT_ROOT/src-tauri/binaries"
mkdir -p "$BIN_DIR"

# Resolve the Rust target triple (argument, else `rustc` host).
TRIPLE="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
[[ -n "$TRIPLE" ]] || { echo "Could not determine target triple." >&2; exit 1; }

# Map Rust triple -> rclone's download slug + file extension.
case "$TRIPLE" in
  x86_64-unknown-linux-gnu)   SLUG="linux-amd64";   EXT="" ;;
  aarch64-unknown-linux-gnu)  SLUG="linux-arm64";   EXT="" ;;
  x86_64-pc-windows-msvc)     SLUG="windows-amd64"; EXT=".exe" ;;
  aarch64-pc-windows-msvc)    SLUG="windows-arm64"; EXT=".exe" ;;
  x86_64-apple-darwin)        SLUG="osx-amd64";     EXT="" ;;
  aarch64-apple-darwin)       SLUG="osx-arm64";     EXT="" ;;
  *) echo "Unsupported triple: $TRIPLE" >&2; exit 1 ;;
esac

DEST="$BIN_DIR/rclone-$TRIPLE$EXT"
URL="https://downloads.rclone.org/rclone-current-$SLUG.zip"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Fetching rclone for $TRIPLE ($SLUG)..."
curl -fsSL "$URL" -o "$TMP/rclone.zip"

# Extract portably: prefer unzip, fall back to Python's zipfile (present on all
# CI runners, including Windows git-bash where `unzip` may be missing).
if command -v unzip >/dev/null 2>&1; then
    unzip -q "$TMP/rclone.zip" -d "$TMP"
elif command -v python3 >/dev/null 2>&1; then
    python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$TMP/rclone.zip" "$TMP"
else
    python -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$TMP/rclone.zip" "$TMP"
fi

# The zip contains a versioned folder like rclone-v1.6x.x-<slug>/rclone[.exe]
FOUND="$(find "$TMP" -type f -name "rclone$EXT" | head -1)"
[[ -n "$FOUND" ]] || { echo "rclone binary not found in archive." >&2; exit 1; }

cp "$FOUND" "$DEST"
chmod +x "$DEST" 2>/dev/null || true
echo "Installed sidecar: $DEST"

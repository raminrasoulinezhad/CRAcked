# CRAcked — Packaging & Installation

CRAcked ships as a **single self-contained installer per OS**. Everything needed
to run is bundled:

- the app itself (Rust + web UI),
- the web engine is handled by the installer (WebView2 auto-installs on Windows,
  WebKitGTK is a declared dependency on the Linux `.deb`, native on macOS),
- **git is embedded** in the app (no `git` install needed for version history),
- **rclone is bundled** as a sidecar (no `rclone` install needed for Google Drive
  backup — the user only does a one-time Google login).

So whoever you share an installer with just runs it; there is nothing else to set up.

---

## For your friend (just installing)

| OS                        | File to download                                                  | How to install                                                |
| ------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------- |
| **Windows**               | `CRAcked_x.y.z_x64-setup.exe` (or `.msi`)                         | Double-click, click through.                                  |
| **macOS**                 | `CRAcked_x.y.z_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) | Open, drag CRAcked to Applications.                           |
| **Linux**                 | `cracked_x.y.z_amd64.AppImage`                                    | `chmod +x` it, then double-click / run. Portable, no install. |
| **Linux (Debian/Ubuntu)** | `cracked_x.y.z_amd64.deb`                                         | `sudo apt install ./cracked_*.deb`                            |

After installing, the app appears in the Start menu / Launchpad / app grid. For
Google Drive backup, follow the one-time login in [`BACKUP.md`](BACKUP.md).

---

## Producing the installers

### Automatically, for all three OSes (recommended) — GitHub Actions

`.github/workflows/release.yml` builds every installer on its native OS and
attaches them to a GitHub Release. One-time setup:

1. Create a GitHub repo and push this code to it.
2. Tag a release and push the tag:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. The workflow builds Linux/Windows/macOS installers and creates a **draft
   Release** with all files attached. Review it and publish.

(You can also trigger it manually from the repo's **Actions** tab via
"Run workflow".)

### Locally, for your current OS

Prerequisites: Rust, Node, and (Linux only) the Tauri system libraries.

```bash
# Fetch the rclone sidecar for this machine's platform
./scripts/fetch-rclone.sh

# Build the installer(s)
cd src-tauri
cargo tauri build            # all default bundles for this OS
# or pick specific ones:
cargo tauri build --bundles deb,appimage   # Linux
cargo tauri build --bundles nsis,msi       # Windows
cargo tauri build --bundles dmg            # macOS
```

Output lands in `src-tauri/target/release/bundle/`.

> You can only build a given OS's installer _on_ that OS. That's why the CI
> pipeline exists — it runs the Windows build on Windows, the Mac build on Mac,
> etc.

---

## For your own machine while developing — live launcher

This installs a launcher that runs the app **straight from this repo, rebuilding
on each launch**, so it always reflects your latest changes:

```bash
./scripts/install-desktop.sh              # install
./scripts/install-desktop.sh --uninstall  # remove
```

Then press **Super** (the "Windows" key) and type **CRAcked**, or double-click
the desktop icon. Each launch does an incremental `cargo build` first, so any
code changes you've made are picked up automatically.

# CRAcked — Backup & Restore

CRAcked keeps your data safe with two layers:

1. **Local git history** — your data directory is a git repository. Every change
   writes a plain-text `snapshot.json` and commits it, so you have the complete,
   tamper-evident version history right on your machine. This needs no setup.
2. **Google Drive mirror (append-only)** — the whole repo is copied to Google
   Drive using `rclone copy`. Because it's *copy* and never *sync*, deleting a
   file locally can **never** delete it from Drive. This layer is optional and
   takes a one-time setup below.

The data directory (which is also the git repo) is:

```
~/.local/share/CRAcked/
├── cracked.db      # the SQLite database the app uses
├── snapshot.json   # human-readable snapshot (full history is in git)
└── .git/           # the version history
```

---

## One-time Google Drive setup

You only do this once. It logs `rclone` into your Google account so CRAcked can
push backups there.

### 1. Configure an rclone remote

Run this in your terminal (in Claude Code, prefix with `!`):

```
rclone config
```

Answer the prompts:

- `n` → **New remote**
- name → **`gdrive`**  (remember this name — you'll type it into the app)
- storage → choose the number for **Google Drive** (`drive`)
- `client_id` / `client_secret` → leave blank (press Enter) — uses rclone's default
- `scope` → choose **`1`** (full access) or **`3`** (drive.file — access only to
  files rclone creates; sufficient and more private). Recommended: **`3`**.
- `root_folder_id`, `service_account_file` → leave blank
- **Edit advanced config?** → `n`
- **Use auto config?** → `y` (opens your browser to log in to Google)
- Log in and allow access in the browser.
- **Configure this as a team drive?** → `n`
- Confirm → `y`, then `q` to quit.

### 2. Test the remote

```
rclone lsd gdrive:
```

If it lists your Drive folders, it's working.

### 3. Tell CRAcked to use it

Open CRAcked → **Backup** section:

- **rclone remote**: `gdrive`
- **Drive folder**: `CRAcked` (or any folder name you like)
- Click **Save backup settings**, then **Back up now**.

You should see "Pushed to Google Drive." A `CRAcked` folder will appear in your
Drive containing the backup.

From then on, every change you make is auto-committed locally and pushed to
Drive in the background.

---

## Restore (new machine, or recovering after a wipe)

1. Install rclone and configure the same `gdrive` remote (steps above).
2. Pull the backup down:

   ```
   rclone copy gdrive:CRAcked ~/.local/share/CRAcked
   ```

3. Launch CRAcked. It reads `cracked.db` from that directory — your data is back,
   with the full git history intact (`git -C ~/.local/share/CRAcked log`).

---

## Notes

- **No encryption**: data is stored on Google Drive as-is (your choice — no
  password to manage). Anyone with access to that Drive folder can read it.
- **Append-only**: `rclone copy` never deletes on the destination, so the Drive
  copy survives local deletion. (Google Drive's own trash/versioning is *not*
  relied upon — the durable history is the git repo itself.)
- **Inspect history any time**:

  ```
  git -C ~/.local/share/CRAcked log --oneline
  git -C ~/.local/share/CRAcked show HEAD:snapshot.json
  ```

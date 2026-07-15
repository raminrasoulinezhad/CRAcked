# CLAUDE.md — working notes for CRAcked

CRAcked is a Tauri 2 desktop app (Rust backend + vanilla JS UI) that tracks
Canadian RRSP / TFSA / FHSA contribution room per family member, with a local
git-versioned data store and append-only Google Drive backup.

Layout: rule engines are pure Rust in `src-tauri/src/{rrsp,tfsa,fhsa}.rs`;
persistence in `db.rs`; Tauri commands in `lib.rs`; backup in `backup.rs`;
frontend in `src/`. Money is always integer **cents**.

## 🔒 Rule-verification discipline (MANDATORY, every relevant commit)

The contribution rules and dollar figures **must stay correct and dated**.
[`RULES.md`](RULES.md) is the authoritative, dated record.

**If a commit changes a rule engine (`rrsp.rs`, `tfsa.rs`, `fhsa.rs`) or any
limit/cap/date, you MUST, in the same commit:**

1. **Re-verify** every affected figure against the current CRA source (see the
   Sources in `RULES.md`) — do not trust memory.
2. **Update `RULES.md`**: the relevant table/rule, the **"Last verified" date**
   at the top, and add a row to the **Verification log**.
3. **Reconcile**: confirm the code and `RULES.md` state exactly the same thing
   (the guard tests in `rrsp.rs` / `tfsa.rs` assert the table values — keep them
   in sync).
4. **Completeness check**: is a year, cap, start date, or window missing or now
   out of date? RRSP dollar-limit table, TFSA annual-limit table, program start
   dates, the FHSA $8k/$40k caps and 15-year window — all must be present and
   current.

A pre-commit hook (`scripts/check-rules-doc.sh`) **blocks** committing rule-engine
edits unless `RULES.md` is also staged. That's a backstop for the steps above,
not a substitute for actually verifying.

Even on commits that don't touch rules, if the "Last verified" date is more than
a few months old, re-verify and refresh it.

## Quality gates (enforced by pre-commit + CI)

- `cargo fmt --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test`
- `prettier --check .` for JS/CSS/HTML/JSON/MD/YAML
- Run `pre-commit run --all-files` before finishing a change.

## Verifying behaviour

`cd src-tauri && cargo test`. To run the app, `./scripts/cracked.sh` (or
`cargo tauri dev`). Data lives in `~/.local/share/CRAcked/`.

<p align="center">
  <img src="logo.png" alt="CRAcked logo" width="280">
</p>

<h1 align="center">CRAcked — Developer Guide</h1>

<p align="center"><strong>Contribution Room, Cracked.</strong><br>
<em>Claw it back. Legally.</em></p>

<p align="center"><sub>Looking to just use the app? See <a href="README.md">README.md</a>.</sub></p>

---

## What is this?

**CRAcked** is a personal tracker for Canada's tax-advantaged registered
accounts — the ones with the confusing, ever-changing contribution rules that
the CRA would rather you not fully understand:

- **RRSP** — Registered Retirement Savings Plan
- **TFSA** — Tax-Free Savings Account
- **FHSA** — First Home Savings Account

Each account grows your contribution room by a different set of rules —
annual limits, carry-forwards, income-based accrual, lifetime caps, and
withdrawal quirks. Miss the details and you either leave room on the table or
get hit with an over-contribution penalty (1% per month — ouch).

CRAcked keeps a running, accurate picture of **how much room you actually
have**, **how much you've used**, and **how much tax you're clawing back**.

## Why

The CRA gives every Canadian a set of legal tools to defer or eliminate tax —
but tracking the room across three accounts, across years, is genuinely
annoying. CRAcked exists to make that dead simple, so you never:

- over-contribute and eat a penalty,
- under-use room you're legally entitled to, or
- lose track of carry-forward across years.

## The accounts, at a glance

| Account  | Tax treatment                                                              | Room grows by                                               | Key caps                                    |
| -------- | -------------------------------------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------- |
| **RRSP** | Tax-**deferred** (deduct now, taxed on withdrawal)                         | 18% of prior-year earned income + unused room               | Annual dollar max; over-contribution buffer |
| **TFSA** | Tax-**free** (no deduction, tax-free growth & withdrawal)                  | Annual limit + unused room + withdrawals re-added next year | Cumulative lifetime room since eligibility  |
| **FHSA** | Tax-**free for a first home** (deduct now, tax-free qualifying withdrawal) | Annual limit + limited carry-forward                        | Annual cap; lifetime cap; 15-year window    |

> Exact numbers change year to year and depend on your personal history —
> CRAcked is designed to model these rules explicitly rather than hard-code a
> single year.

## Features

- [x] RRSP room (18% accrual, annual dollar-limit cap, unused-room carry-forward, $2,000 buffer)
- [x] TFSA room (cumulative annual room, withdrawal re-added next year, no buffer)
- [x] FHSA room ($8k/year, $40k lifetime, capped carry-forward, 15-year window)
- [x] Per-family-member tracking; deletable records with confirmation
- [x] Current-year estimated-income pro-rated accrual projection (RRSP)
- [x] Local git version history + append-only Google Drive backup
- [ ] Estimate tax refund / deferral from RRSP & FHSA deductions
- [ ] Multi-year projections

## Tech stack

- **Desktop:** [Tauri 2](https://tauri.app) — Rust backend, static HTML/CSS/JS UI
- **Rule engine:** pure Rust (`src-tauri/src/rrsp.rs`), fully unit-tested
- **Storage:** SQLite (bundled, `src-tauri/src/db.rs`) — money stored as integer cents
- **Backup:** the data directory is a git repo; `rclone copy` mirrors it
  append-only to Google Drive (see [`BACKUP.md`](BACKUP.md))

## Installing it

CRAcked ships as a **single self-contained installer per OS** — the web engine,
an embedded git, and a bundled `rclone` are all included, so there's nothing else
to install. See [`PACKAGING.md`](PACKAGING.md) for downloads and build steps.

- **Windows**: `.exe` / `.msi` — double-click.
- **macOS**: `.dmg` — drag to Applications.
- **Linux**: `.AppImage` (portable) or `.deb` (`sudo apt install ./cracked_*.deb`).

Cross-platform installers are produced by the GitHub Actions release workflow
(`.github/workflows/release.yml`) — tag a release (`git tag v0.1.0 && git push
origin v0.1.0`) and it builds all three.

### Develop with a live launcher

Install a launcher that runs the app from this repo and rebuilds on each launch,
so it always reflects your latest code — then find it via the **Super/Windows key**:

```bash
./scripts/install-desktop.sh
```

## Running from source

Prerequisites: Rust ≥ 1.77, Node, and the Tauri Linux system libraries
(WebKitGTK etc.).

```bash
cd src-tauri && cargo tauri dev     # dev run
cd src-tauri && cargo test          # Rust unit + integration tests
npm ci && npm test                  # frontend unit + e2e tests (vitest/jsdom)
./scripts/fetch-rclone.sh           # grab the rclone sidecar, then:
cd src-tauri && cargo tauri build   # build an installer for this OS
```

Your data lives in `~/.local/share/CRAcked/`. To back it up to Google Drive,
follow the one-time setup in [`BACKUP.md`](BACKUP.md).

## Code quality gates

All enforced identically by **pre-commit** (locally) and **GitHub Actions**
(`.github/workflows/ci.yml`) — nothing merges that fails these:

| Check          | Command                                                 |
| -------------- | ------------------------------------------------------- |
| Rust format    | `cargo fmt --check` (config: `rustfmt.toml`)            |
| Rust lint      | `cargo clippy --all-targets -- -D warnings`             |
| Rust tests     | `cargo test` — unit + integration (push / CI)           |
| Frontend tests | `npm test` — vitest unit + jsdom e2e (push / CI)        |
| Frontend/docs  | `npm run format:check` (prettier, config `.prettierrc`) |
| File hygiene   | trailing whitespace, EOF, YAML/JSON/TOML, etc.          |

One-time setup for contributors:

```bash
pip install pre-commit
pre-commit install                      # fmt / clippy / prettier / hygiene on commit
pre-commit install --hook-type pre-push # full test suite on push
```

Money is stored in integer **cents** everywhere, written with a deliberate
`8_000_00` (= $8,000.00) digit grouping. Rule engines (`rrsp`/`tfsa`/`fhsa`)
are pure and fully unit-tested; the CRA limit tables are guarded by tests that
assert the exact published figures.

## Testing

Three tiers, all run in CI (`.github/workflows/ci.yml`) and on `git push` via
pre-commit:

| Tier            | Where                                                          | What it covers                                                                                                                                                                            |
| --------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Unit**        | `#[cfg(test)]` in each `*.rs`; `src/__tests__/helpers.test.js` | Rule engines on hand-built inputs; the CRA limit-table guards; pure JS helpers (`toCents`, `fmt`, …).                                                                                     |
| **Integration** | `src-tauri/tests/integration.rs`                               | The whole Rust backend the way the Tauri commands drive it: real (in-memory) SQLite → `db` writes → `*_year_data` shaping → engine `compute` → the numbers the UI shows.                  |
| **E2E**         | `src/__tests__/app.e2e.test.js`                                | The frontend end-to-end in jsdom: real `index.html` markup + a mocked Tauri IPC bridge, driving `init()` and user actions (form submits, tab switches) and asserting on the rendered DOM. |

```bash
cd src-tauri && cargo test   # Rust unit + integration
npm test                     # frontend unit + e2e (vitest run)
npm run test:watch           # vitest in watch mode while developing the UI
```

The frontend e2e stops at the IPC boundary (it mocks `invoke`) rather than
launching a real WebView — it needs no display server, so it stays fast and
non-flaky in CI. A full WebDriver-driven run of the packaged app
(`tauri-driver`) is a possible future addition on top of this.

## Status

🚧 Active development. **All three accounts (RRSP, TFSA, FHSA) are implemented**
with per-family-member tracking, room calculation, over-contribution warnings,
and local + Google Drive backup. 63 Rust tests (unit + integration) and 13
frontend tests (unit + jsdom e2e); CI + pre-commit enforced.

## Disclaimer

CRAcked is a personal tracking tool, **not** tax advice. Contribution rules
are set by the Canada Revenue Agency and change over time. Always verify your
own limits against your CRA My Account and, when in doubt, consult a tax
professional.

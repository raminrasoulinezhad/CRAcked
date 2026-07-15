<p align="center">
  <img src="logo.png" alt="CRAcked logo" width="260">
</p>

<h1 align="center">CRAcked</h1>

<p align="center"><strong>Contribution Room, Cracked.</strong><br>
<em>Claw it back. Legally.</em></p>

---

CRAcked is a simple desktop app that keeps track of your Canadian
tax-advantaged accounts — **RRSP**, **TFSA**, and **FHSA** — so you always know
exactly how much you're allowed to contribute, and never get surprised by an
over-contribution penalty.

The rules the CRA uses to calculate your contribution room are genuinely
confusing: they build up over years, carry forward, and change annually. CRAcked
does that math for you and shows you one clear number: **how much room you have
left.**

> 🇨🇦 Your money stays on your computer. Nothing is sent anywhere unless *you*
> turn on Google Drive backup.

---

## What it does

- 📊 **Shows your contribution room at a glance** — one headline number for how
  much you can still put in.
- 🧮 **Does the CRA math automatically** — earns new room from your income,
  carries forward what you didn't use, and adds it all up across years.
- ⚠️ **Warns you before you over-contribute** — including the $2,000 buffer and
  the 1%-per-month penalty estimate if you go over.
- 📝 **Logs every contribution** with date, amount, and a note.
- 🕓 **Keeps a full history of your data** — every change is saved, so you can
  always look back.
- 👪 **Tracks your whole family** — add each person; everyone gets their own
  RRSP, TFSA, and FHSA. Switch between them with the “Family member” selector.
- ☁️ **Optional Google Drive backup** — safe, automatic, and set up in two
  minutes (see below).

> **All three accounts are supported:** RRSP, TFSA, and FHSA — each with its own
> rules (RRSP's income-based room and $2,000 buffer, TFSA's withdrawal
> re-contribution timing, FHSA's $8k/year and $40k lifetime caps).

---

## Installing

Download the installer for your system and run it — that's the whole setup.
Everything the app needs is included in the one file.

| Your computer | Download this | Then |
| --- | --- | --- |
| **Windows** | `CRAcked_..._x64-setup.exe` | Double-click and follow the prompts. |
| **Mac (Apple Silicon)** | `CRAcked_..._aarch64.dmg` | Open it, drag CRAcked into Applications. |
| **Mac (Intel)** | `CRAcked_..._x64.dmg` | Open it, drag CRAcked into Applications. |
| **Linux** | `CRAcked_..._amd64.AppImage` | Make it executable, then double-click. |
| **Linux (Ubuntu/Debian)** | `CRAcked_..._amd64.deb` | Install with your software centre, or `sudo apt install ./CRAcked_*.deb`. |

Once installed, find **CRAcked** in your Start menu (Windows), Launchpad (Mac),
or app grid (Linux) — press the **Windows key / Command** and type "CRAcked".

<sub>On Windows you may see a "Windows protected your PC" notice for an unsigned
app — click **More info → Run anyway**. On Mac, if it won't open, right-click the
app → **Open**. This is normal for indie apps that aren't code-signed.</sub>

---

## How to use it

Pick a family member from the **Family member** selector at the top (add more
with **+ Add**). Each person has their own RRSP, TFSA, and FHSA tabs. The
**Backup** tab covers everyone at once.

For RRSP, CRAcked builds your contribution room from two things: **your income**
(which earns you room) and **your contributions** (which use it up). Here's the flow:

### 1. (Optional) Enter your starting room
If you've had an RRSP before using this app, open the **Starting room** section
and enter the unused room from your latest **CRA Notice of Assessment**. Starting
fresh? Leave it at $0.

### 2. Add your earned income, year by year
In **Annual earned income**, enter what you earned each year.

> 💡 Income earned in one year gives you room the **following** year. RRSP room =
> 18% of the prior year's income, up to that year's cap. CRAcked handles this
> timing for you — just enter each year's income.

### 3. Log your contributions
In **Add a contribution**, enter the tax year, date, and amount each time you put
money in. They show up in the **Contribution log**, where you can delete mistakes.

### 4. Read your dashboard
- **Contribution room left** — the big number: what you can still contribute.
- **Year-by-year room** — how room built up and got used each year.
- **Warnings** — if you've gone over your limit, CRAcked tells you by how much
  and the estimated monthly penalty.

That's it. Your data saves automatically as you go.

---

## Setting up Google Drive backup (optional)

By default, your data is saved on your computer with a full history. If you'd
also like an automatic, safe copy in your own Google Drive, it takes about two
minutes to set up once.

> 🔒 Backups are **append-only**: even if you delete data on your computer, the
> Drive copy is never removed. Your data is stored on Drive as-is (not
> encrypted), so keep that folder private to your account.

**Quick version:**

1. Open a terminal and run `rclone config` once, then log into your Google
   account (it opens your browser — just click *Allow*). Name the connection
   **`gdrive`**.
   <br><sub>CRAcked bundles rclone for its own backups. If your terminal says
   `rclone` isn't found, grab the free 2-minute download from
   [rclone.org/downloads](https://rclone.org/downloads/).</sub>
2. In CRAcked's **Backup** section, type `gdrive` as the remote and click
   **Save**, then **Back up now**.

From then on, every change is backed up to a **CRAcked** folder in your Drive
automatically.

👉 **Full step-by-step instructions (with every prompt explained) are in
[BACKUP.md](BACKUP.md).**

### Getting your data back
Lost your computer or starting on a new one? Install CRAcked, set up the same
`gdrive` connection, and copy the backup down — your data and its full history
come right back. See [BACKUP.md](BACKUP.md#restore-new-machine-or-recovering-after-a-wipe).

---

## Where your data lives

Your data is stored in a `CRAcked` folder in your user data directory:

- **Windows**: `%APPDATA%\CRAcked\`
- **Mac**: `~/Library/Application Support/CRAcked/`
- **Linux**: `~/.local/share/CRAcked/`

It's a small database file plus a readable `snapshot.json`. You can copy this
folder anywhere as a manual backup at any time.

---

## Good to know

- **This isn't tax advice.** CRAcked is a personal tracking tool. Contribution
  rules and dollar limits are set by the CRA and change over time — always check
  your numbers against your **CRA My Account**, and consult a professional if
  you're unsure.
- **Verify the yearly limits.** CRAcked knows the published RRSP dollar limits
  through recent years and marks any year it's unsure about as an estimate.
- **Free & private.** No accounts, no tracking, no data leaves your machine
  unless you turn on Drive backup.

---

<p align="center"><sub>Developer? See <a href="DEVELOPERS_README.md">DEVELOPERS_README.md</a> for how to build and contribute.</sub></p>

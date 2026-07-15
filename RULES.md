# CRAcked — Implemented Contribution Rules

This document is the **authoritative, dated record** of every Canadian
contribution rule CRAcked implements. It exists so the rules the app enforces
are transparent, auditable, and pinned to a point in time.

> ## ⚠️ Rules change; this snapshot does not
>
> Every figure and rule below reflects the Canada Revenue Agency (CRA) rules **as
> understood on the "Last verified" date**. Tax law and dollar limits change. If
> the CRA changes a rule after that date and CRAcked has not been updated,
> **the app will be wrong, and CRAcked accepts no responsibility** for decisions
> made on out-of-date figures. Always confirm against your **CRA My Account** and
> a tax professional. CRAcked is a personal tracking tool, **not tax advice**.

**Last verified:** 2026-07-15
**Verified against:** CRA "MP, DB, RRSP, DPSP, ALDA, TFSA limits" table, via
taxtips.ca (which mirrors the CRA figures) and corroborating sources (see
[Sources](#sources)).

All money in CRAcked is stored and computed in **integer cents** to avoid
floating-point error.

---

## RRSP — Registered Retirement Savings Plan

- **Program:** long-standing (since 1957). CRAcked models annual dollar limits
  from **2010** onward.
- **New room for a year** =
  `min( 18% of the PRIOR year's earned income , that year's annual dollar limit ) − pension adjustment`,
  floored at 0.
- **Timing:** income earned in year _Y_ drives room for year _Y+1_.
- **Carry-forward:** unused room carries forward **indefinitely**.
- **Over-contribution buffer:** **$2,000** lifetime cushion (for those 18+).
- **Penalty:** **1% per month** on the cumulative excess _beyond_ the $2,000 buffer.

### RRSP annual dollar limits (implemented)

| Year | Limit   | Year | Limit   |
| ---- | ------- | ---- | ------- |
| 2010 | $22,000 | 2019 | $26,500 |
| 2011 | $22,450 | 2020 | $27,230 |
| 2012 | $22,970 | 2021 | $27,830 |
| 2013 | $23,820 | 2022 | $29,210 |
| 2014 | $24,270 | 2023 | $30,780 |
| 2015 | $24,930 | 2024 | $31,560 |
| 2016 | $25,370 | 2025 | $32,490 |
| 2017 | $26,010 | 2026 | $33,810 |
| 2018 | $26,230 |      |         |

For a year with no built-in limit that the user has already reached, CRAcked
treats it as an **error** and requires the user to enter the CRA figure (stored
as a per-year override).

### RRSP — not modelled (deliberately, for now)

Pension adjustment reversals (PAR), past-service pension adjustments (PSPA), the
first-60-days deduction-timing rule, the age-71 contribution cutoff, and
HBP/LLP withdrawals. `earned_income` is taken as a user input.

---

## TFSA — Tax-Free Savings Account

- **Program start:** **2009**.
- **Eligibility:** age 18+ and a Canadian resident. Room accrues every year from
  the year both are true (earliest 2009) — the user sets this "start year".
- **Room:** a fixed annual dollar amount, **cumulative**, carried forward
  **indefinitely**.
- **Withdrawals:** the amount withdrawn is added back to room on **January 1 of
  the following year** — not in the year of withdrawal.
- **Buffer:** **none**. Any excess is penalised.
- **Penalty:** **1% per month** on the excess.
- Annual limits are indexed to inflation and rounded to the nearest **$500**.

### TFSA annual limits (implemented)

| Year | Limit   | Year | Limit  |
| ---- | ------- | ---- | ------ |
| 2009 | $5,000  | 2018 | $5,500 |
| 2010 | $5,000  | 2019 | $6,000 |
| 2011 | $5,000  | 2020 | $6,000 |
| 2012 | $5,000  | 2021 | $6,000 |
| 2013 | $5,500  | 2022 | $6,000 |
| 2014 | $5,500  | 2023 | $6,500 |
| 2015 | $10,000 | 2024 | $7,000 |
| 2016 | $5,500  | 2025 | $7,000 |
| 2017 | $5,500  | 2026 | $7,000 |

**Cumulative room 2009–2026 (eligible the whole time): $109,000.**

For years beyond the table, CRAcked uses the most recent known limit as an
estimate and flags it.

---

## FHSA — First Home Savings Account

- **Program start:** **April 1, 2023**.
- **Room start:** begins the year the user **opens** their first FHSA (not before).
- **Annual grant:** **$8,000** per year, granted in full at the start of the year
  — **not pro-rated by month** (it's time-based and fixed, independent of income).
  The current year's full $8,000 is available immediately once the account is open.
- **Lifetime limit:** **$40,000** total of granted room.
- **Carry-forward:** unused room carries forward, but the amount applied in any
  one year is **capped at $8,000** — so the most contributable in a single year is
  **$16,000** ($8,000 current + $8,000 carried).
- **Withdrawals:** do **not** restore contribution room (unlike TFSA). A
  qualifying first-home withdrawal is simply tax-free.
- **Buffer:** **none**. **Penalty: 1% per month** on the excess.
- **Participation window:** the FHSA must be closed/transferred by the earliest of
  **15 years** after opening, the end of the year you turn **71**, or the year
  after your **first qualifying withdrawal**. CRAcked flags the **15-year** limit.

### FHSA — not modelled (deliberately, for now)

The age-71 and first-qualifying-withdrawal ends of the participation window
(only the 15-year limit is flagged), and the conditions that make a withdrawal
"qualifying".

---

## Cross-cutting

- **Penalty display** is an estimate of **one month's** 1% charge on the current
  excess; the real CRA penalty accrues for each month the excess remains.
- **Multi-person:** each family member's room is tracked independently.
- **RRSP dollar limits are shared** across people (they're a CRA fact); each
  person's income, contributions, withdrawals, and account settings are their own.

---

## Sources

- CRA — MP, DB, RRSP, DPSP, ALDA, TFSA limits, YMPE and the YAMPE:
  <https://www.canada.ca/en/revenue-agency/services/tax/registered-plans-administrators/pspa/mp-rrsp-dpsp-tfsa-limits-ympe.html>
- TaxTips.ca — RRSP/PRPP contribution limits:
  <https://www.taxtips.ca/rrsp/rrsp-mpp-dpsp-contribution-limits.htm>
- CRA — Design of the Tax-Free First Home Savings Account (FHSA rules).
- CRA — Tax-Free Savings Account (TFSA) contribution room.

---

## Verification log

Add a row every time the rules are re-checked or changed. **Any commit that
edits a rule engine (`rrsp.rs`, `tfsa.rs`, `fhsa.rs`) or a limit table must
update this log and the "Last verified" date at the top** (see `CLAUDE.md`).

| Date       | Change                                                                                  |
| ---------- | --------------------------------------------------------------------------------------- |
| 2026-07-15 | Initial rules documented; RRSP (2010–2026) and TFSA (2009–2026) tables verified vs CRA. |

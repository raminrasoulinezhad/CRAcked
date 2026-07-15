//! TFSA contribution-room rule engine.
//!
//! Pure logic — no database, no Tauri. Money is integer **cents**.
//!
//! ## The rules modelled (Canada Revenue Agency)
//!
//! - You accrue a fixed **annual dollar amount** of room every year you are 18+
//!   and a Canadian resident, starting in 2009 (or the year you turned 18, if
//!   later). Unused room **carries forward indefinitely**.
//! - **Withdrawals** do NOT free up room in the year you withdraw. The amount
//!   you took out is added back to your room on **January 1 of the following
//!   year**. Re-contributing it too early is an over-contribution.
//! - There is **no buffer** (unlike RRSP's $2,000). Any excess is penalised at
//!   **1% per month** on the excess.
//!
//! The app supplies each year's `eligible` flag (derived from the user's TFSA
//! start year) and the contribution / withdrawal totals.

use serde::{Deserialize, Serialize};

pub type Cents = i64;

/// TFSA monthly over-contribution penalty rate: 1% per month.
pub const MONTHLY_PENALTY_RATE_PERCENT: i64 = 1;

/// Published TFSA annual dollar limits, in whole dollars. VERIFY against the CRA
/// and extend as new years are announced.
const ANNUAL_DOLLAR_LIMITS: &[(i32, i64)] = &[
    (2009, 5_000),
    (2010, 5_000),
    (2011, 5_000),
    (2012, 5_000),
    (2013, 5_500),
    (2014, 5_500),
    (2015, 10_000),
    (2016, 5_500),
    (2017, 5_500),
    (2018, 5_500),
    (2019, 6_000),
    (2020, 6_000),
    (2021, 6_000),
    (2022, 6_000),
    (2023, 6_500),
    (2024, 7_000),
    (2025, 7_000),
    (2026, 7_000),
];

/// The published TFSA annual limit for `year` in cents, or `None` if we have no
/// figure for that year yet.
pub fn annual_dollar_limit(year: i32) -> Option<Cents> {
    ANNUAL_DOLLAR_LIMITS
        .iter()
        .find(|(y, _)| *y == year)
        .map(|(_, dollars)| *dollars * 100)
}

/// The most recent published limit, used as an estimate for years beyond the
/// table (flagged so the UI can warn).
fn latest_known_limit() -> Cents {
    ANNUAL_DOLLAR_LIMITS
        .iter()
        .max_by_key(|(y, _)| *y)
        .map(|(_, d)| *d * 100)
        .unwrap_or(0)
}

/// Per-year inputs for the TFSA engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YearData {
    pub year: i32,
    /// Whether the person accrued room this year (18+, resident, year >= 2009).
    pub eligible: bool,
    #[serde(default)]
    pub contribution: Cents,
    #[serde(default)]
    pub withdrawal: Cents,
}

/// Computed room picture for a single TFSA year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YearComputation {
    pub year: i32,
    /// New room accrued this year (0 if not eligible).
    pub new_room: Cents,
    /// Room added back from the PRIOR year's withdrawals (the Jan-1 re-add).
    pub withdrawals_readded: Cents,
    /// Unused room carried into this year.
    pub opening_room: Cents,
    /// Room available to contribute this year
    /// (`opening_room + new_room + withdrawals_readded`).
    pub available_room: Cents,
    pub contribution: Cents,
    /// Amount withdrawn this year (frees room only NEXT year).
    pub withdrawal: Cents,
    /// Unused room carried out. Negative = cumulative over-contribution.
    pub closing_room: Cents,
    /// Cumulative over-contribution (0 if within room; there is no buffer).
    pub over_contribution: Cents,
    /// Estimated penalty for ONE month on the current excess (1%).
    pub estimated_monthly_penalty: Cents,
    /// True when the year's dollar limit was estimated (beyond the known table).
    pub dollar_limit_missing: bool,
}

/// Run the full multi-year computation. `opening_room` is unused room carried in
/// before the first year in `years` (normally 0 when the data starts at the
/// eligibility year). `years` is sorted ascending defensively.
pub fn compute(years: &[YearData], opening_room: Cents) -> Vec<YearComputation> {
    let mut sorted: Vec<&YearData> = years.iter().collect();
    sorted.sort_by_key(|d| d.year);

    let mut carry = opening_room;
    let mut prev_withdrawal: Cents = 0;
    let mut out = Vec::with_capacity(sorted.len());

    for data in sorted {
        let (new_room, limit_missing) = if data.eligible {
            match annual_dollar_limit(data.year) {
                Some(limit) => (limit, false),
                None => (latest_known_limit(), true),
            }
        } else {
            (0, false)
        };

        let withdrawals_readded = prev_withdrawal;
        let opening_room = carry;
        let available_room = opening_room + new_room + withdrawals_readded;
        let closing_room = available_room - data.contribution;

        let over = (-closing_room).max(0);
        let penalty = over * MONTHLY_PENALTY_RATE_PERCENT / 100;

        out.push(YearComputation {
            year: data.year,
            new_room,
            withdrawals_readded,
            opening_room,
            available_room,
            contribution: data.contribution,
            withdrawal: data.withdrawal,
            closing_room,
            over_contribution: over,
            estimated_monthly_penalty: penalty,
            dollar_limit_missing: limit_missing,
        });

        carry = closing_room;
        prev_withdrawal = data.withdrawal;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(dollars: i64) -> Cents {
        dollars * 100
    }

    fn year(y: i32, contribution: Cents, withdrawal: Cents) -> YearData {
        YearData { year: y, eligible: true, contribution, withdrawal }
    }

    #[test]
    fn room_accumulates_from_annual_limits() {
        // 2021 ($6,000) + 2022 ($6,000), no contributions.
        let years = vec![year(2021, 0, 0), year(2022, 0, 0)];
        let r = compute(&years, 0);
        assert_eq!(r[0].available_room, d(6_000));
        assert_eq!(r[0].closing_room, d(6_000));
        assert_eq!(r[1].opening_room, d(6_000));
        assert_eq!(r[1].available_room, d(12_000));
    }

    #[test]
    fn ineligible_years_accrue_nothing() {
        let years = vec![
            YearData { year: 2020, eligible: false, contribution: 0, withdrawal: 0 },
            year(2021, 0, 0),
        ];
        let r = compute(&years, 0);
        assert_eq!(r[0].new_room, 0);
        assert_eq!(r[1].new_room, d(6_000));
    }

    #[test]
    fn withdrawal_is_readded_the_following_year_only() {
        // 2021: room $6,000, contribute $6,000 (room now 0), withdraw $4,000.
        // 2022: withdrawal re-added -> $4,000 + new $6,000 = $10,000 available.
        let years = vec![year(2021, d(6_000), d(4_000)), year(2022, 0, 0)];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, 0); // withdrawal does NOT restore room this year
        assert_eq!(r[0].withdrawal, d(4_000));
        assert_eq!(r[1].withdrawals_readded, d(4_000));
        assert_eq!(r[1].available_room, d(10_000));
    }

    #[test]
    fn recontributing_a_withdrawal_same_year_is_over_contribution() {
        // 2021: room $6,000, contribute $6,000, withdraw $2,000, then re-contribute
        // that $2,000 the SAME year -> total $8,000 contributed, $2,000 over.
        let years = vec![year(2021, d(8_000), d(2_000))];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, d(-2_000));
        assert_eq!(r[0].over_contribution, d(2_000)); // no buffer
        assert_eq!(r[0].estimated_monthly_penalty, d(20)); // 1% of $2,000
    }

    #[test]
    fn no_buffer_unlike_rrsp() {
        // Over by just $100 -> immediately penalised (RRSP would allow $2,000).
        let years = vec![year(2021, d(6_100), 0)];
        let r = compute(&years, 0);
        assert_eq!(r[0].over_contribution, d(100));
        assert_eq!(r[0].estimated_monthly_penalty, 100); // 1% of $100 = $1.00
    }

    #[test]
    fn future_year_uses_latest_limit_and_flags_estimate() {
        let years = vec![YearData { year: 2099, eligible: true, contribution: 0, withdrawal: 0 }];
        let r = compute(&years, 0);
        assert!(r[0].dollar_limit_missing);
        assert_eq!(r[0].new_room, latest_known_limit());
    }
}

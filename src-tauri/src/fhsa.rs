//! FHSA (First Home Savings Account) contribution-room rule engine.
//!
//! Pure logic — no database, no Tauri. Money is integer **cents**.
//!
//! ## The rules modelled (Canada Revenue Agency)
//!
//! - Room starts accruing the year you **open** your first FHSA (not before).
//! - You get **$8,000** of new room each year, up to a **$40,000 lifetime**
//!   total of granted room.
//! - Unused room carries forward, but the carry-forward applied to a year is
//!   **capped at $8,000**. So the most you can contribute in one year is
//!   `$8,000 (this year) + $8,000 (carried) = $16,000`.
//! - **No buffer**: excess is penalised at **1% per month**.
//! - Withdrawals do **not** restore contribution room (unlike TFSA) — a
//!   qualifying first-home withdrawal is simply tax-free.
//! - The account has a participation window (~15 years from opening / age 71);
//!   we flag years beyond it rather than blocking them.

use serde::{Deserialize, Serialize};

pub type Cents = i64;

pub const ANNUAL_GRANT: Cents = 8_000_00; // $8,000
pub const LIFETIME_LIMIT: Cents = 40_000_00; // $40,000
pub const CARRYFORWARD_CAP: Cents = 8_000_00; // max unused room applied from prior year
pub const MONTHLY_PENALTY_RATE_PERCENT: i64 = 1;
/// Years the account may participate before it must be closed/transferred.
pub const PARTICIPATION_YEARS: i32 = 15;

/// Per-year inputs for the FHSA engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YearData {
    pub year: i32,
    /// Whether the FHSA was open this year (year >= opening year).
    pub open: bool,
    #[serde(default)]
    pub contribution: Cents,
    /// Tracked for the user's information; does NOT affect room.
    #[serde(default)]
    pub withdrawal: Cents,
}

/// Computed room picture for a single FHSA year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YearComputation {
    pub year: i32,
    /// New room granted this year (0 once the $40k lifetime grant is exhausted,
    /// or before the account is open).
    pub new_room: Cents,
    /// Unused room carried in from the prior year, after the $8,000 cap.
    pub carryforward_in: Cents,
    /// Room available to contribute this year (`carryforward_in + new_room`).
    pub available_room: Cents,
    pub contribution: Cents,
    pub withdrawal: Cents,
    /// Unused room at year end (before next year's carry-forward cap). Negative
    /// = over-contribution.
    pub closing_room: Cents,
    /// Cumulative contributions to date.
    pub lifetime_contributed: Cents,
    /// Remaining lifetime grant room (`$40,000 − granted so far`).
    pub lifetime_remaining: Cents,
    pub over_contribution: Cents,
    pub estimated_monthly_penalty: Cents,
    /// True once the year is past the ~15-year participation window.
    pub past_participation_window: bool,
}

/// Run the full multi-year computation. `open_year` is the year the first FHSA
/// was opened. Years before `open_year` grant no room. `years` is sorted
/// ascending defensively.
pub fn compute(years: &[YearData], open_year: i32) -> Vec<YearComputation> {
    let mut sorted: Vec<&YearData> = years.iter().collect();
    sorted.sort_by_key(|d| d.year);

    let mut granted_total: Cents = 0;
    let mut lifetime_contributed: Cents = 0;
    let mut carry: Cents = 0; // raw unused room from prior year (may be negative)
    let mut out = Vec::with_capacity(sorted.len());

    for data in sorted {
        // New grant: $8,000/year while open, capped by the remaining lifetime grant.
        let new_room = if data.open {
            ANNUAL_GRANT.min(LIFETIME_LIMIT - granted_total).max(0)
        } else {
            0
        };

        // Carry-forward applied is capped at $8,000 (a negative carry — i.e. a
        // prior over-contribution — still propagates in full to keep the deficit).
        let carryforward_in = carry.min(CARRYFORWARD_CAP);
        let available_room = carryforward_in + new_room;
        let closing_room = available_room - data.contribution;

        granted_total += new_room;
        lifetime_contributed += data.contribution;

        let over = (-closing_room).max(0);
        let penalty = over * MONTHLY_PENALTY_RATE_PERCENT / 100;

        out.push(YearComputation {
            year: data.year,
            new_room,
            carryforward_in,
            available_room,
            contribution: data.contribution,
            withdrawal: data.withdrawal,
            closing_room,
            lifetime_contributed,
            lifetime_remaining: (LIFETIME_LIMIT - granted_total).max(0),
            over_contribution: over,
            estimated_monthly_penalty: penalty,
            past_participation_window: data.open && data.year >= open_year + PARTICIPATION_YEARS,
        });

        carry = closing_room;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(dollars: i64) -> Cents {
        dollars * 100
    }

    fn year(y: i32, contribution: Cents) -> YearData {
        YearData {
            year: y,
            open: true,
            contribution,
            withdrawal: 0,
        }
    }

    #[test]
    fn grants_8000_per_year() {
        let years = vec![year(2023, 0), year(2024, 0)];
        let r = compute(&years, 2023);
        assert_eq!(r[0].new_room, d(8_000));
        assert_eq!(r[0].available_room, d(8_000));
        assert_eq!(r[1].new_room, d(8_000));
    }

    #[test]
    fn carryforward_is_capped_at_8000() {
        // Open 2023, contribute nothing for two years. Unused after 2023 = $8,000,
        // after 2024 = $16,000. But 2025's applied carry-forward is capped at
        // $8,000, so 2025 available = $8,000 (carry) + $8,000 (new) = $16,000,
        // NOT $16,000 + $8,000.
        let years = vec![year(2023, 0), year(2024, 0), year(2025, 0)];
        let r = compute(&years, 2023);
        assert_eq!(r[1].closing_room, d(16_000));
        assert_eq!(r[2].carryforward_in, d(8_000)); // capped
        assert_eq!(r[2].available_room, d(16_000));
    }

    #[test]
    fn lifetime_grant_caps_at_40000() {
        // Six open years would grant 6*8,000 = $48,000 of raw annual, but the
        // lifetime grant stops at $40,000.
        let years: Vec<YearData> = (2023..2029).map(|y| year(y, 0)).collect();
        let r = compute(&years, 2023);
        let total_granted: Cents = r.iter().map(|y| y.new_room).sum();
        assert_eq!(total_granted, d(40_000));
        assert_eq!(r.last().unwrap().lifetime_remaining, 0);
        // The 6th year grants nothing (already at lifetime cap after 5 years).
        assert_eq!(r[5].new_room, 0);
    }

    #[test]
    fn contributions_use_room_and_carry_the_remainder() {
        // 2023: available $8,000, contribute $3,000 -> $5,000 unused.
        // 2024: carry $5,000 + new $8,000 = $13,000 available.
        let years = vec![year(2023, d(3_000)), year(2024, 0)];
        let r = compute(&years, 2023);
        assert_eq!(r[0].closing_room, d(5_000));
        assert_eq!(r[1].carryforward_in, d(5_000));
        assert_eq!(r[1].available_room, d(13_000));
        assert_eq!(r[1].lifetime_contributed, d(3_000));
    }

    #[test]
    fn over_contribution_penalised_no_buffer() {
        // 2023: available $8,000, contribute $9,000 -> $1,000 over.
        let years = vec![year(2023, d(9_000))];
        let r = compute(&years, 2023);
        assert_eq!(r[0].closing_room, d(-1_000));
        assert_eq!(r[0].over_contribution, d(1_000));
        assert_eq!(r[0].estimated_monthly_penalty, d(10)); // 1% of $1,000
    }

    #[test]
    fn years_before_opening_grant_nothing() {
        let years = vec![
            YearData {
                year: 2022,
                open: false,
                contribution: 0,
                withdrawal: 0,
            },
            year(2023, 0),
        ];
        let r = compute(&years, 2023);
        assert_eq!(r[0].new_room, 0);
        assert_eq!(r[1].new_room, d(8_000));
    }

    #[test]
    fn flags_years_past_participation_window() {
        let years: Vec<YearData> = (2023..=2023 + PARTICIPATION_YEARS)
            .map(|y| year(y, 0))
            .collect();
        let r = compute(&years, 2023);
        assert!(!r[0].past_participation_window);
        assert!(r.last().unwrap().past_participation_window); // 2023 + 15 = 2038
    }

    #[test]
    fn empty_input_produces_no_rows() {
        assert!(compute(&[], 2023).is_empty());
    }

    #[test]
    fn max_16000_in_a_year_with_full_carryforward() {
        // Idle first year banks $8,000; year two allows $8,000 carry + $8,000 new.
        let years = vec![year(2023, 0), year(2024, d(16_000))];
        let r = compute(&years, 2023);
        assert_eq!(r[1].available_room, d(16_000));
        assert_eq!(r[1].closing_room, 0);
        assert_eq!(r[1].over_contribution, 0);
        assert_eq!(r[1].lifetime_contributed, d(16_000));
    }

    #[test]
    fn contributing_past_lifetime_cap_is_over_contribution() {
        // Contribute the full $8,000 for five years = $40,000 (the lifetime cap),
        // then any 6th-year contribution has no new grant to draw on.
        let mut years: Vec<YearData> = (2023..2028).map(|y| year(y, d(8_000))).collect();
        years.push(year(2028, d(1_000)));
        let r = compute(&years, 2023);
        assert_eq!(r[4].lifetime_contributed, d(40_000));
        assert_eq!(r[5].new_room, 0); // lifetime grant exhausted
        assert_eq!(r[5].available_room, 0);
        assert_eq!(r[5].over_contribution, d(1_000));
    }

    #[test]
    fn withdrawals_do_not_restore_room() {
        // Contribute $8,000, withdraw $5,000 the same year: room is unaffected.
        let years = vec![YearData {
            year: 2023,
            open: true,
            contribution: d(8_000),
            withdrawal: d(5_000),
        }];
        let r = compute(&years, 2023);
        assert_eq!(r[0].closing_room, 0); // withdrawal didn't add room back
        assert_eq!(r[0].withdrawal, d(5_000));
    }

    #[test]
    fn over_contribution_then_recovery_next_year() {
        // Year 1: contribute $10,000 (over by $2,000). Year 2: $8,000 new grant,
        // minus the $2,000 carried deficit -> $6,000 available.
        let years = vec![year(2023, d(10_000)), year(2024, 0)];
        let r = compute(&years, 2023);
        assert_eq!(r[0].over_contribution, d(2_000));
        assert_eq!(r[0].closing_room, d(-2_000));
        assert_eq!(r[1].carryforward_in, d(-2_000));
        assert_eq!(r[1].available_room, d(6_000));
        assert_eq!(r[1].over_contribution, 0);
    }
}

//! RRSP contribution-room rule engine.
//!
//! Pure logic — no database, no Tauri. Everything here is unit-testable in
//! isolation. Money is represented as integer **cents** (`Cents = i64`) so we
//! never accumulate floating-point rounding error on people's finances.
//!
//! ## The rules modelled (Canada Revenue Agency)
//!
//! Your RRSP room for a year is built from two parts:
//!
//! 1. **New room earned this year** =
//!    `min( 18% of PRIOR year's earned income , annual dollar limit ) − pension adjustment`
//!    (floored at 0).
//! 2. **Unused room carried forward** from all prior years (indefinitely).
//!
//! `Available room = carried-forward unused room + new room`.
//!
//! **Over-contribution:** the CRA allows a **$2,000 lifetime cushion**. Cumulative
//! contributions beyond `room + $2,000` attract a penalty tax of **1% per month**
//! on the excess.
//!
//! Not modelled in this v1 (kept explicit so we can add them later): pension
//! adjustment reversals (PAR), past-service pension adjustments (PSPA), the
//! first-60-days deduction-timing rule, and the age-71 contribution cutoff.
//! `earned_income` is taken as an input (its full CRA definition is out of scope
//! for the engine — the app supplies the number).

use serde::{Deserialize, Serialize};

/// Money in whole cents. $1.00 == 100.
pub type Cents = i64;

/// The tax-free over-contribution cushion the CRA permits (for those 18+): $2,000.
pub const OVER_CONTRIBUTION_BUFFER: Cents = 2_000_00;

/// Monthly penalty rate on the excess over the buffer: 1% per month.
pub const MONTHLY_PENALTY_RATE_PERCENT: i64 = 1;

/// Published RRSP annual dollar limits (the maximum *new* room a year can grant,
/// regardless of income), in whole dollars.
///
/// VERIFY against the CRA and extend as new years are announced — these change
/// every year. Source: CRA "MP, DB, RRSP, DPSP, ALDA, TFSA limits" table.
const ANNUAL_DOLLAR_LIMITS: &[(i32, i64)] = &[
    (2018, 26_230),
    (2019, 26_500),
    (2020, 27_230),
    (2021, 27_830),
    (2022, 29_210),
    (2023, 30_780),
    (2024, 31_560),
    (2025, 32_490),
    (2026, 33_810),
];

/// The annual RRSP dollar limit for `year`, in cents, or `None` if we don't have
/// a published figure for that year yet (the caller should surface a warning
/// rather than silently guess).
pub fn annual_dollar_limit(year: i32) -> Option<Cents> {
    ANNUAL_DOLLAR_LIMITS
        .iter()
        .find(|(y, _)| *y == year)
        .map(|(_, dollars)| *dollars * 100)
}

/// Per-year inputs the app collects. One entry per calendar year.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YearData {
    pub year: i32,
    /// Earned income from the **prior** year — this is what drives *this* year's
    /// new room (18% of the previous year's earned income).
    pub prior_year_earned_income: Cents,
    /// Pension adjustment reported on a T4 (employer pension plans). 0 if none.
    #[serde(default)]
    pub pension_adjustment: Cents,
    /// Total contributed toward this year's room.
    #[serde(default)]
    pub contribution: Cents,
}

/// The computed room picture for a single year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YearComputation {
    pub year: i32,
    /// New room earned this year (after the dollar cap and pension adjustment).
    pub new_room: Cents,
    /// Unused room carried into this year from prior years.
    pub opening_room: Cents,
    /// Total room available to contribute this year (`opening_room + new_room`).
    pub available_room: Cents,
    /// Amount actually contributed this year.
    pub contribution: Cents,
    /// Unused room carried out of this year. Negative means cumulative
    /// over-contribution.
    pub closing_room: Cents,
    /// Cumulative over-contribution beyond the $2,000 buffer (0 if within it).
    pub over_contribution: Cents,
    /// Estimated penalty for ONE month on the current excess (1% of the excess).
    /// The real CRA penalty accrues per month the excess remains.
    pub estimated_monthly_penalty: Cents,
    /// True when we had no published dollar limit for this year, so `new_room`
    /// was computed from income alone (uncapped) — treat as an estimate.
    pub dollar_limit_missing: bool,
}

/// Round `income * 18%` to the nearest cent using integer math (income is
/// non-negative cents, so no float rounding drift).
fn eighteen_percent(income: Cents) -> Cents {
    let income = income.max(0);
    (income * 18 + 50) / 100
}

/// Compute the new room a year grants, and whether the dollar limit was known.
fn new_room_for_year(data: &YearData) -> (Cents, bool) {
    let income_based = eighteen_percent(data.prior_year_earned_income);
    let (capped, limit_known) = match annual_dollar_limit(data.year) {
        Some(limit) => (income_based.min(limit), true),
        None => (income_based, false),
    };
    let room = (capped - data.pension_adjustment.max(0)).max(0);
    (room, limit_known)
}

/// Run the full multi-year computation.
///
/// `opening_unused_room` is the unused RRSP room carried in *before* the first
/// year in `years` (e.g. what the CRA shows you as of the start of your history
/// in the app). Use 0 if starting from scratch. `years` should be sorted
/// ascending by year; this function sorts defensively.
pub fn compute(years: &[YearData], opening_unused_room: Cents) -> Vec<YearComputation> {
    let mut sorted: Vec<&YearData> = years.iter().collect();
    sorted.sort_by_key(|d| d.year);

    let mut carry = opening_unused_room;
    let mut out = Vec::with_capacity(sorted.len());

    for data in sorted {
        let (new_room, limit_known) = new_room_for_year(data);
        let opening_room = carry;
        let available_room = opening_room + new_room;
        let closing_room = available_room - data.contribution;

        // Over-contribution is measured on the cumulative shortfall beyond the
        // buffer. closing_room already carries prior years' excess forward.
        let excess = (-closing_room - OVER_CONTRIBUTION_BUFFER).max(0);
        let penalty = excess * MONTHLY_PENALTY_RATE_PERCENT / 100;

        out.push(YearComputation {
            year: data.year,
            new_room,
            opening_room,
            available_room,
            contribution: data.contribution,
            closing_room,
            over_contribution: excess,
            estimated_monthly_penalty: penalty,
            dollar_limit_missing: !limit_known,
        });

        carry = closing_room;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dollars(d: i64) -> Cents {
        d * 100
    }

    #[test]
    fn eighteen_percent_rounds_to_nearest_cent() {
        // 18% of $50,000 = $9,000.00
        assert_eq!(eighteen_percent(dollars(50_000)), dollars(9_000));
        // 18% of $12,345.67 = $2,222.2206 -> rounds to $2,222.22
        assert_eq!(eighteen_percent(1_234_567), 222_222);
    }

    #[test]
    fn new_room_is_capped_by_annual_dollar_limit() {
        // 18% of $250,000 = $45,000, but 2024 limit is $31,560.
        let d = YearData {
            year: 2024,
            prior_year_earned_income: dollars(250_000),
            pension_adjustment: 0,
            contribution: 0,
        };
        let (room, known) = new_room_for_year(&d);
        assert!(known);
        assert_eq!(room, dollars(31_560));
    }

    #[test]
    fn pension_adjustment_reduces_new_room() {
        // 18% of $60,000 = $10,800; PA of $4,000 -> $6,800 new room.
        let d = YearData {
            year: 2024,
            prior_year_earned_income: dollars(60_000),
            pension_adjustment: dollars(4_000),
            contribution: 0,
        };
        let (room, _) = new_room_for_year(&d);
        assert_eq!(room, dollars(6_800));
    }

    #[test]
    fn unused_room_carries_forward() {
        let years = vec![
            YearData {
                year: 2023,
                prior_year_earned_income: dollars(50_000), // 18% = $9,000 (< 2023 cap)
                pension_adjustment: 0,
                contribution: dollars(4_000),
            },
            YearData {
                year: 2024,
                prior_year_earned_income: dollars(50_000), // another $9,000
                pension_adjustment: 0,
                contribution: dollars(2_000),
            },
        ];
        let r = compute(&years, 0);
        // 2023: room 9,000, contribute 4,000 -> carry 5,000.
        assert_eq!(r[0].available_room, dollars(9_000));
        assert_eq!(r[0].closing_room, dollars(5_000));
        // 2024: opening 5,000 + new 9,000 = 14,000 available, contribute 2,000.
        assert_eq!(r[1].opening_room, dollars(5_000));
        assert_eq!(r[1].available_room, dollars(14_000));
        assert_eq!(r[1].closing_room, dollars(12_000));
        assert_eq!(r[1].over_contribution, 0);
    }

    #[test]
    fn within_2000_buffer_has_no_penalty() {
        // Room $9,000, contribute $10,500 -> $1,500 over, within the $2,000 buffer.
        let years = vec![YearData {
            year: 2024,
            prior_year_earned_income: dollars(50_000),
            pension_adjustment: 0,
            contribution: dollars(10_500),
        }];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, dollars(-1_500));
        assert_eq!(r[0].over_contribution, 0);
        assert_eq!(r[0].estimated_monthly_penalty, 0);
    }

    #[test]
    fn excess_beyond_buffer_incurs_one_percent_monthly() {
        // Room $9,000, contribute $12,000 -> $3,000 over; $1,000 beyond buffer.
        let years = vec![YearData {
            year: 2024,
            prior_year_earned_income: dollars(50_000),
            pension_adjustment: 0,
            contribution: dollars(12_000),
        }];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, dollars(-3_000));
        assert_eq!(r[0].over_contribution, dollars(1_000));
        assert_eq!(r[0].estimated_monthly_penalty, dollars(10)); // 1% of $1,000
    }

    #[test]
    fn opening_room_from_prior_history_is_respected() {
        // User carries $20,000 of unused room from before the tracked period.
        let years = vec![YearData {
            year: 2024,
            prior_year_earned_income: dollars(50_000), // +$9,000 new room
            pension_adjustment: 0,
            contribution: dollars(25_000),
        }];
        let r = compute(&years, dollars(20_000));
        assert_eq!(r[0].opening_room, dollars(20_000));
        assert_eq!(r[0].available_room, dollars(29_000));
        assert_eq!(r[0].closing_room, dollars(4_000));
    }

    #[test]
    fn unknown_year_is_flagged_and_uncapped() {
        // Year far in the future with no published limit.
        let d = YearData {
            year: 2099,
            prior_year_earned_income: dollars(300_000),
            pension_adjustment: 0,
            contribution: 0,
        };
        let (room, known) = new_room_for_year(&d);
        assert!(!known);
        assert_eq!(room, dollars(54_000)); // uncapped 18%
        let r = compute(&[d], 0);
        assert!(r[0].dollar_limit_missing);
    }

    #[test]
    fn out_of_order_years_are_sorted() {
        let years = vec![
            YearData { year: 2024, prior_year_earned_income: dollars(50_000), pension_adjustment: 0, contribution: 0 },
            YearData { year: 2023, prior_year_earned_income: dollars(50_000), pension_adjustment: 0, contribution: 0 },
        ];
        let r = compute(&years, 0);
        assert_eq!(r[0].year, 2023);
        assert_eq!(r[1].year, 2024);
    }
}

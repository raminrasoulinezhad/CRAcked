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
//!    (floored at 0). The **annual dollar limit** is the hard cap that turns
//!    "18% of income" into a bounded number, and it changes every year.
//! 2. **Unused room carried forward** from all prior years (indefinitely).
//!
//! `Available room = carried-forward unused room + new room`.
//!
//! **Over-contribution:** the CRA allows a **$2,000 lifetime cushion**. Cumulative
//! contributions beyond `room + $2,000` attract a penalty tax of **1% per month**
//! on the excess.
//!
//! The annual dollar limit is passed in per year ([`YearData::dollar_limit`]),
//! already resolved by the caller from a user override or the built-in
//! [`annual_dollar_limit`] table. When it is `None` the year's limit is unknown
//! and the result is flagged (`dollar_limit_missing`) so the app can require the
//! user to supply it rather than guessing.
//!
//! Not modelled in this v1: pension adjustment reversals (PAR), past-service
//! pension adjustments (PSPA), the first-60-days deduction-timing rule, and the
//! age-71 contribution cutoff. `earned_income` is taken as an input.

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
/// These are CRA-published figures and **must be correct** — verify before
/// adding a new year. Source: CRA "MP, DB, RRSP, DPSP, ALDA, TFSA limits".
/// Verified 2026-07-15 against taxtips.ca (mirrors the CRA table). See RULES.md.
const ANNUAL_DOLLAR_LIMITS: &[(i32, i64)] = &[
    (2010, 22_000),
    (2011, 22_450),
    (2012, 22_970),
    (2013, 23_820),
    (2014, 24_270),
    (2015, 24_930),
    (2016, 25_370),
    (2017, 26_010),
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

/// The built-in RRSP annual dollar limit for `year`, in cents, or `None` if the
/// program ships no figure for that year (the caller must then require the user
/// to supply it, or treat future years as projections).
pub fn annual_dollar_limit(year: i32) -> Option<Cents> {
    ANNUAL_DOLLAR_LIMITS
        .iter()
        .find(|(y, _)| *y == year)
        .map(|(_, dollars)| *dollars * 100)
}

/// The most recent year for which a built-in limit exists (used to explain how
/// current the shipped data is).
pub fn latest_known_limit_year() -> i32 {
    ANNUAL_DOLLAR_LIMITS
        .iter()
        .map(|(y, _)| *y)
        .max()
        .unwrap_or(0)
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
    /// The RRSP annual dollar limit for THIS year, in cents, already resolved
    /// (user override → built-in table). `None` means it is unknown.
    #[serde(default)]
    pub dollar_limit: Option<Cents>,
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
    pub estimated_monthly_penalty: Cents,
    /// True when this year's dollar limit was unknown, so `new_room` was computed
    /// from income alone (uncapped) — the app should require the real figure.
    pub dollar_limit_missing: bool,
}

/// Round `income * 18%` to the nearest cent using integer math.
fn eighteen_percent(income: Cents) -> Cents {
    let income = income.max(0);
    (income * 18 + 50) / 100
}

/// Compute the new room that a given prior-year income would generate under a
/// given dollar limit. Returns `(room, limit_known)`. Used for the current-year
/// projection where the income is an estimate. `dollar_limit` is the limit for
/// the year the room applies to.
pub fn new_room(
    prior_year_income: Cents,
    dollar_limit: Option<Cents>,
    pension_adjustment: Cents,
) -> (Cents, bool) {
    new_room_for_year(&YearData {
        year: 0,
        prior_year_earned_income: prior_year_income,
        pension_adjustment,
        contribution: 0,
        dollar_limit,
    })
}

/// Compute the new room a year grants, and whether the dollar limit was known.
fn new_room_for_year(data: &YearData) -> (Cents, bool) {
    let income_based = eighteen_percent(data.prior_year_earned_income);
    let (capped, limit_known) = match data.dollar_limit {
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

    /// Build a YearData with the built-in dollar limit resolved (mirrors how the
    /// DB layer supplies it in production).
    fn yd(year: i32, income: Cents, pa: Cents, contribution: Cents) -> YearData {
        YearData {
            year,
            prior_year_earned_income: income,
            pension_adjustment: pa,
            contribution,
            dollar_limit: annual_dollar_limit(year),
        }
    }

    #[test]
    fn eighteen_percent_rounds_to_nearest_cent() {
        assert_eq!(eighteen_percent(dollars(50_000)), dollars(9_000));
        assert_eq!(eighteen_percent(1_234_567), 222_222);
    }

    #[test]
    fn built_in_table_matches_cra_verified_values() {
        // Full table, verified 2026-07 against the CRA / taxtips.ca figures.
        // A guard against accidental edits — every value must stay correct.
        let expected = [
            (2010, 22_000),
            (2011, 22_450),
            (2012, 22_970),
            (2013, 23_820),
            (2014, 24_270),
            (2015, 24_930),
            (2016, 25_370),
            (2017, 26_010),
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
        for (year, amount) in expected {
            assert_eq!(
                annual_dollar_limit(year),
                Some(dollars(amount)),
                "RRSP {year}"
            );
        }
        assert_eq!(annual_dollar_limit(2009), None); // before the table
        assert_eq!(latest_known_limit_year(), 2026);
    }

    #[test]
    fn new_room_is_capped_by_annual_dollar_limit() {
        // 18% of $250,000 = $45,000, but 2024 limit is $31,560.
        let d = yd(2024, dollars(250_000), 0, 0);
        let (room, known) = new_room_for_year(&d);
        assert!(known);
        assert_eq!(room, dollars(31_560));
    }

    #[test]
    fn pension_adjustment_reduces_new_room() {
        // 18% of $60,000 = $10,800; PA of $4,000 -> $6,800 new room.
        let d = yd(2024, dollars(60_000), dollars(4_000), 0);
        let (room, _) = new_room_for_year(&d);
        assert_eq!(room, dollars(6_800));
    }

    #[test]
    fn unused_room_carries_forward() {
        let years = vec![
            yd(2023, dollars(50_000), 0, dollars(4_000)),
            yd(2024, dollars(50_000), 0, dollars(2_000)),
        ];
        let r = compute(&years, 0);
        assert_eq!(r[0].available_room, dollars(9_000));
        assert_eq!(r[0].closing_room, dollars(5_000));
        assert_eq!(r[1].opening_room, dollars(5_000));
        assert_eq!(r[1].available_room, dollars(14_000));
        assert_eq!(r[1].closing_room, dollars(12_000));
        assert_eq!(r[1].over_contribution, 0);
    }

    #[test]
    fn within_2000_buffer_has_no_penalty() {
        let years = vec![yd(2024, dollars(50_000), 0, dollars(10_500))];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, dollars(-1_500));
        assert_eq!(r[0].over_contribution, 0);
        assert_eq!(r[0].estimated_monthly_penalty, 0);
    }

    #[test]
    fn excess_beyond_buffer_incurs_one_percent_monthly() {
        let years = vec![yd(2024, dollars(50_000), 0, dollars(12_000))];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, dollars(-3_000));
        assert_eq!(r[0].over_contribution, dollars(1_000));
        assert_eq!(r[0].estimated_monthly_penalty, dollars(10));
    }

    #[test]
    fn opening_room_from_prior_history_is_respected() {
        let years = vec![yd(2024, dollars(50_000), 0, dollars(25_000))];
        let r = compute(&years, dollars(20_000));
        assert_eq!(r[0].opening_room, dollars(20_000));
        assert_eq!(r[0].available_room, dollars(29_000));
        assert_eq!(r[0].closing_room, dollars(4_000));
    }

    #[test]
    fn a_user_supplied_limit_overrides_when_built_in_is_absent() {
        // 2099 has no built-in limit; caller supplies one -> it caps room.
        let d = YearData {
            year: 2099,
            prior_year_earned_income: dollars(300_000),
            pension_adjustment: 0,
            contribution: 0,
            dollar_limit: Some(dollars(40_000)),
        };
        let (room, known) = new_room_for_year(&d);
        assert!(known);
        assert_eq!(room, dollars(40_000));
    }

    #[test]
    fn unknown_limit_is_flagged_and_uncapped() {
        let d = YearData {
            year: 2099,
            prior_year_earned_income: dollars(300_000),
            pension_adjustment: 0,
            contribution: 0,
            dollar_limit: None,
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
            yd(2024, dollars(50_000), 0, 0),
            yd(2023, dollars(50_000), 0, 0),
        ];
        let r = compute(&years, 0);
        assert_eq!(r[0].year, 2023);
        assert_eq!(r[1].year, 2024);
    }

    #[test]
    fn empty_input_produces_no_rows() {
        assert!(compute(&[], 0).is_empty());
        assert!(compute(&[], dollars(5_000)).is_empty());
    }

    #[test]
    fn zero_income_grants_no_new_room() {
        let r = compute(&[yd(2024, 0, 0, 0)], 0);
        assert_eq!(r[0].new_room, 0);
        assert_eq!(r[0].available_room, 0);
    }

    #[test]
    fn pension_adjustment_exceeding_18pct_floors_at_zero() {
        // 18% of $40,000 = $7,200; PA of $10,000 would go negative -> floored to 0.
        let (room, _) = new_room_for_year(&yd(2024, dollars(40_000), dollars(10_000), 0));
        assert_eq!(room, 0);
    }

    #[test]
    fn buffer_boundary_is_exact_to_the_cent() {
        // Exactly $2,000 over -> still within buffer, no penalty.
        let r = compute(&[yd(2024, dollars(50_000), 0, dollars(11_000))], 0);
        assert_eq!(r[0].closing_room, dollars(-2_000));
        assert_eq!(r[0].over_contribution, 0);
        // One cent past the buffer -> 1 cent of excess.
        let r = compute(
            &[YearData {
                year: 2024,
                prior_year_earned_income: dollars(50_000),
                pension_adjustment: 0,
                contribution: dollars(11_000) + 1,
                dollar_limit: annual_dollar_limit(2024),
            }],
            0,
        );
        assert_eq!(r[0].over_contribution, 1);
    }

    #[test]
    fn over_contribution_persists_then_recovers_next_year() {
        // 2024: room $9,000, contribute $15,000 -> -$6,000 (over by $4,000 past buffer).
        // 2025: +$9,000 new room -> closing -$6,000 + $9,000 = $3,000, recovered.
        let years = vec![
            yd(2024, dollars(50_000), 0, dollars(15_000)),
            yd(2025, dollars(50_000), 0, 0),
        ];
        let r = compute(&years, 0);
        assert_eq!(r[0].closing_room, dollars(-6_000));
        assert_eq!(r[0].over_contribution, dollars(4_000));
        assert_eq!(r[1].opening_room, dollars(-6_000));
        assert_eq!(r[1].closing_room, dollars(3_000));
        assert_eq!(r[1].over_contribution, 0);
    }
}

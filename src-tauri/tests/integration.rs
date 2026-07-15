//! End-to-end **integration** tests for the storage + rule-engine stack.
//!
//! Unlike the in-module unit tests (which exercise each rule engine on
//! hand-built `YearData`), these drive the *whole* Rust backend the way the
//! Tauri commands in `lib.rs` do: open a real (in-memory) SQLite database,
//! write through the `db` API, shape the per-year data with
//! `db::{rrsp,tfsa,fhsa}_year_data`, run the engine `compute`, and assert on the
//! numbers the UI would actually show. If a schema change, a data-shaping bug,
//! or an engine change breaks the pipeline, these catch it even when the
//! isolated unit tests still pass.

use cracked_lib::{db, fhsa, rrsp, tfsa};
use rusqlite::Connection;

const D: i64 = 100; // dollars → cents multiplier

/// Fresh in-memory DB with the default person; returns `(conn, person_id)`.
fn fixture() -> (Connection, i64) {
    let conn = db::open_in_memory().expect("open in-memory db");
    let me = db::ensure_default_person(&conn).expect("default person");
    (conn, me)
}

// ---------------------------------------------------------------------------
// RRSP: income → new room → contributions → closing room, through the DB.
// ---------------------------------------------------------------------------

#[test]
fn rrsp_multi_year_room_carries_forward_through_the_db() {
    let (conn, me) = fixture();

    // Prior-year income drives the *following* year's new room.
    // 2022 income $50k → 2023 new room = 18% = $9,000 (under the 2023 cap).
    // 2023 income $60k → 2024 new room = 18% = $10,800 (under the 2024 cap).
    db::upsert_annual_income(
        &conn,
        me,
        &db::AnnualIncome {
            year: 2022,
            earned_income_cents: 50_000 * D,
            pension_adjustment_cents: 0,
            is_estimate: false,
        },
    )
    .unwrap();
    db::upsert_annual_income(
        &conn,
        me,
        &db::AnnualIncome {
            year: 2023,
            earned_income_cents: 60_000 * D,
            pension_adjustment_cents: 0,
            is_estimate: false,
        },
    )
    .unwrap();

    db::add_contribution(&conn, me, "RRSP", 2023, "2023-03-01", 4_000 * D, "").unwrap();
    db::add_contribution(&conn, me, "RRSP", 2024, "2024-03-01", 2_000 * D, "").unwrap();

    let data = db::rrsp_year_data(&conn, me).unwrap();
    let years = rrsp::compute(&data, db::get_rrsp_opening_room(&conn, me).unwrap());

    assert_eq!(years.len(), 2, "years 2023 and 2024");

    let y2023 = &years[0];
    assert_eq!(y2023.year, 2023);
    assert_eq!(y2023.new_room, 9_000 * D);
    assert_eq!(y2023.opening_room, 0);
    assert_eq!(y2023.closing_room, 5_000 * D); // 9,000 available − 4,000

    let y2024 = &years[1];
    assert_eq!(y2024.year, 2024);
    assert_eq!(y2024.new_room, 10_800 * D);
    assert_eq!(y2024.opening_room, 5_000 * D); // carried from 2023
    assert_eq!(y2024.available_room, 15_800 * D);
    assert_eq!(y2024.closing_room, 13_800 * D); // 15,800 − 2,000

    // Summary numbers the command returns.
    let current_room = years.last().unwrap().closing_room;
    let total: i64 = years.iter().map(|y| y.contribution).sum();
    assert_eq!(current_room, 13_800 * D);
    assert_eq!(total, 6_000 * D);
    assert!(years.iter().all(|y| !y.dollar_limit_missing));
}

#[test]
fn rrsp_over_contribution_penalty_surfaces_through_the_db() {
    let (conn, me) = fixture();

    // 2023 income $50k → 2024 new room $9,000. Contribute $12,000 in 2024:
    // closing −$3,000; $2,000 buffer absorbed; $1,000 excess → $10/month penalty.
    db::upsert_annual_income(
        &conn,
        me,
        &db::AnnualIncome {
            year: 2023,
            earned_income_cents: 50_000 * D,
            pension_adjustment_cents: 0,
            is_estimate: false,
        },
    )
    .unwrap();
    db::add_contribution(&conn, me, "RRSP", 2024, "2024-06-01", 12_000 * D, "lump").unwrap();

    let data = db::rrsp_year_data(&conn, me).unwrap();
    let years = rrsp::compute(&data, 0);
    let last = years.last().unwrap();

    assert_eq!(last.closing_room, -3_000 * D);
    assert_eq!(last.over_contribution, 1_000 * D);
    assert_eq!(last.estimated_monthly_penalty, 10 * D);
}

#[test]
fn rrsp_user_dollar_limit_override_is_applied_end_to_end() {
    let (conn, me) = fixture();

    // 2098 income high enough that 18% ($54k) would exceed any cap.
    db::upsert_annual_income(
        &conn,
        me,
        &db::AnnualIncome {
            year: 2098,
            earned_income_cents: 300_000 * D,
            pension_adjustment_cents: 0,
            is_estimate: false,
        },
    )
    .unwrap();

    // No built-in limit for 2099: without an override the year is uncapped/flagged.
    let flagged = rrsp::compute(&db::rrsp_year_data(&conn, me).unwrap(), 0);
    assert!(
        flagged.last().unwrap().dollar_limit_missing,
        "2099 has no shipped limit"
    );
    assert_eq!(flagged.last().unwrap().new_room, 54_000 * D); // uncapped 18%

    // Supply a user override for 2099 → it caps the room and clears the flag.
    db::upsert_rrsp_dollar_limit(&conn, 2099, 40_000 * D).unwrap();
    let capped = rrsp::compute(&db::rrsp_year_data(&conn, me).unwrap(), 0);
    let y = capped.last().unwrap();
    assert!(!y.dollar_limit_missing);
    assert_eq!(y.new_room, 40_000 * D);
}

// ---------------------------------------------------------------------------
// TFSA: withdrawal re-add across the Jan-1 boundary, through the DB.
// ---------------------------------------------------------------------------

#[test]
fn tfsa_withdrawal_is_readded_the_following_year_through_the_db() {
    let (conn, me) = fixture();
    db::set_tfsa_settings(&conn, me, 2023, 0).unwrap();

    // 2023: contribute the full $6,500 room, then withdraw $2,000.
    db::add_contribution(&conn, me, "TFSA", 2023, "2023-02-01", 6_500 * D, "").unwrap();
    db::add_withdrawal(&conn, me, "TFSA", 2023, "2023-11-01", 2_000 * D, "").unwrap();

    let data = db::tfsa_year_data(&conn, me, 2024).unwrap();
    let years = tfsa::compute(&data, db::get_tfsa_opening_room(&conn, me).unwrap());
    assert_eq!(years.len(), 2, "2023 and 2024");

    let y2023 = &years[0];
    assert_eq!(y2023.new_room, 6_500 * D);
    assert_eq!(y2023.closing_room, 0);
    assert_eq!(y2023.withdrawal, 2_000 * D);

    let y2024 = &years[1];
    assert_eq!(y2024.new_room, 7_000 * D);
    assert_eq!(
        y2024.withdrawals_readded,
        2_000 * D,
        "prior-year withdrawal"
    );
    assert_eq!(y2024.available_room, 9_000 * D); // 0 + 7,000 + 2,000
    assert_eq!(y2024.closing_room, 9_000 * D);

    let total_withdrawn: i64 = years.iter().map(|y| y.withdrawal).sum();
    assert_eq!(total_withdrawn, 2_000 * D);
}

// ---------------------------------------------------------------------------
// FHSA: $8k/yr grant, $40k lifetime, and over-contribution — through the DB.
// ---------------------------------------------------------------------------

#[test]
fn fhsa_lifetime_grant_tracks_across_years_through_the_db() {
    let (conn, me) = fixture();
    db::set_fhsa_open_year(&conn, me, 2023).unwrap();

    db::add_contribution(&conn, me, "FHSA", 2023, "2023-05-01", 8_000 * D, "").unwrap();
    db::add_contribution(&conn, me, "FHSA", 2024, "2024-05-01", 8_000 * D, "").unwrap();

    let data = db::fhsa_year_data(&conn, me, 2024).unwrap();
    let open_year = db::get_fhsa_open_year(&conn, me).unwrap().unwrap();
    let years = fhsa::compute(&data, open_year);
    let last = years.last().unwrap();

    assert_eq!(last.lifetime_contributed, 16_000 * D);
    assert_eq!(last.lifetime_remaining, 24_000 * D); // 40,000 − 16,000 granted
    assert_eq!(last.closing_room, 0);
    assert!(!last.past_participation_window);
}

#[test]
fn fhsa_over_contribution_penalty_surfaces_through_the_db() {
    let (conn, me) = fixture();
    db::set_fhsa_open_year(&conn, me, 2024).unwrap();
    // $10,000 into a first year that grants only $8,000 → $2,000 over, $20/mo.
    db::add_contribution(&conn, me, "FHSA", 2024, "2024-01-15", 10_000 * D, "").unwrap();

    let data = db::fhsa_year_data(&conn, me, 2024).unwrap();
    let years = fhsa::compute(&data, 2024);
    let last = years.last().unwrap();
    assert_eq!(last.closing_room, -2_000 * D);
    assert_eq!(last.over_contribution, 2_000 * D);
    assert_eq!(last.estimated_monthly_penalty, 20 * D);
}

// ---------------------------------------------------------------------------
// Cross-cutting: account isolation, person isolation, and snapshot export.
// ---------------------------------------------------------------------------

#[test]
fn contributions_are_isolated_by_account() {
    let (conn, me) = fixture();
    db::add_contribution(&conn, me, "RRSP", 2024, "2024-01-01", 1_000 * D, "").unwrap();
    db::add_contribution(&conn, me, "TFSA", 2024, "2024-01-01", 2_000 * D, "").unwrap();
    db::add_contribution(&conn, me, "FHSA", 2024, "2024-01-01", 3_000 * D, "").unwrap();

    assert_eq!(db::list_contributions(&conn, me, "RRSP").unwrap().len(), 1);
    assert_eq!(
        db::list_contributions(&conn, me, "TFSA").unwrap()[0].amount_cents,
        2_000 * D
    );
    assert_eq!(
        db::list_contributions(&conn, me, "FHSA").unwrap()[0].amount_cents,
        3_000 * D
    );
}

#[test]
fn deleting_a_person_removes_their_data_but_not_others() {
    let (conn, me) = fixture();
    let spouse = db::add_person(&conn, "Alex").unwrap();

    db::add_contribution(&conn, me, "RRSP", 2024, "2024-01-01", 1_000 * D, "").unwrap();
    db::add_contribution(&conn, spouse, "RRSP", 2024, "2024-01-01", 9_000 * D, "").unwrap();

    db::delete_person(&conn, spouse).unwrap();

    assert_eq!(db::list_persons(&conn).unwrap().len(), 1);
    assert_eq!(db::list_contributions(&conn, me, "RRSP").unwrap().len(), 1);
    assert!(db::list_contributions(&conn, spouse, "RRSP")
        .unwrap()
        .is_empty());
}

#[test]
fn export_json_snapshot_reflects_written_data() {
    let (conn, me) = fixture();
    db::set_tfsa_settings(&conn, me, 2020, 0).unwrap();
    db::add_contribution(&conn, me, "TFSA", 2024, "2024-01-01", 5_000 * D, "note").unwrap();
    db::add_withdrawal(&conn, me, "TFSA", 2024, "2024-06-01", 1_000 * D, "").unwrap();

    let snap = db::export_json(&conn).unwrap();
    assert_eq!(snap["schema_version"], 3);
    assert_eq!(snap["persons"].as_array().unwrap().len(), 1);
    assert_eq!(snap["contributions"].as_array().unwrap().len(), 1);
    assert_eq!(snap["withdrawals"].as_array().unwrap().len(), 1);
    assert_eq!(snap["contributions"][0]["amount_cents"], 5_000 * D);
    assert_eq!(snap["contributions"][0]["account"], "TFSA");
}

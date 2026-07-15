//! CRAcked — Tauri application entry point and command surface.
//!
//! The Rust side owns the SQLite database ([`db`]) and the contribution-room
//! rule engines ([`rrsp`], [`tfsa`], [`fhsa`]). The web frontend calls the
//! `#[tauri::command]` functions below via `invoke(...)`.
//!
//! Data is scoped per family member (`person_id`); backup is global.

mod backup;
mod db;
mod fhsa;
mod rrsp;
mod tfsa;

use backup::{BackupConfig, BackupReport};
use db::{AnnualIncome, Person};
use rrsp::{Cents, YearComputation};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};

/// Shared application state: the open database connection behind a mutex.
struct AppState {
    db: Mutex<rusqlite::Connection>,
}

fn to_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Auto-commit the current state to the data git repo (and push to Drive if
/// configured), without blocking the UI. Called after every mutation.
fn auto_backup(conn: &rusqlite::Connection, message: &str) {
    let cfg = BackupConfig::load(conn);
    backup::back_up_async(conn, cfg, message.to_string());
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_persons(state: State<AppState>) -> Result<Vec<Person>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::ensure_default_person(&conn).map_err(to_err)?;
    db::list_persons(&conn).map_err(to_err)
}

#[tauri::command]
fn add_person(state: State<AppState>, name: String) -> Result<i64, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let id = db::add_person(&conn, name.trim()).map_err(to_err)?;
    auto_backup(&conn, &format!("Add person {name}"));
    Ok(id)
}

#[tauri::command]
fn rename_person(state: State<AppState>, id: i64, name: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::rename_person(&conn, id, name.trim()).map_err(to_err)?;
    auto_backup(&conn, &format!("Rename person #{id}"));
    Ok(())
}

#[tauri::command]
fn delete_person(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::delete_person(&conn, id).map_err(to_err)?;
    db::ensure_default_person(&conn).map_err(to_err)?;
    auto_backup(&conn, &format!("Delete person #{id}"));
    Ok(())
}

// ---------------------------------------------------------------------------
// RRSP
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RrspSummary {
    years: Vec<YearComputation>,
    current_room: Cents,
    total_contributed: Cents,
    current_over_contribution: Cents,
    opening_room: Cents,
    missing_limit_years: Vec<i32>,
    latest_built_in_year: i32,
    /// Forward-looking projection of the room the current year is building
    /// (from an estimated or actual current-year income). Informational only —
    /// it does NOT change `current_room`.
    projection: Option<RrspProjection>,
}

/// The current year's accruing-room projection.
#[derive(Debug, Serialize)]
struct RrspProjection {
    /// The year whose income drives this (the current year).
    income_year: i32,
    /// The year this new room actually becomes available (income_year + 1).
    applies_to_year: i32,
    income: Cents,
    is_estimate: bool,
    /// Full-year new room this income would generate.
    projected_new_room: Cents,
    /// Pro-rated to how far through the year we are (`projected × fraction`).
    accrued_to_date: Cents,
    /// Fraction of the current year elapsed (0..1), as supplied by the UI.
    elapsed_fraction: f64,
    /// True if we had no dollar limit for `applies_to_year` (room uncapped).
    dollar_limit_missing: bool,
}

#[tauri::command]
fn get_rrsp_summary(
    state: State<AppState>,
    person_id: i64,
    current_year: i32,
    elapsed_fraction: f64,
) -> Result<RrspSummary, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let opening_room = db::get_rrsp_opening_room(&conn, person_id).map_err(to_err)?;
    let data = db::rrsp_year_data(&conn, person_id).map_err(to_err)?;
    let years = rrsp::compute(&data, opening_room);

    let current_room = years.last().map(|y| y.closing_room).unwrap_or(opening_room);
    let current_over_contribution = years.last().map(|y| y.over_contribution).unwrap_or(0);
    let total_contributed = years.iter().map(|y| y.contribution).sum();
    let missing_limit_years = years
        .iter()
        .filter(|y| y.dollar_limit_missing && y.year <= current_year)
        .map(|y| y.year)
        .collect();

    // Projection: the current year's income (estimated or actual) builds room
    // for next year; pro-rate it to how far through the year we are.
    let overrides = db::list_rrsp_dollar_limit_overrides(&conn).map_err(to_err)?;
    let projection = db::get_annual_income(&conn, person_id, current_year)
        .map_err(to_err)?
        .map(|inc| {
            let applies_to = current_year + 1;
            let limit = db::resolve_rrsp_limit(&overrides, applies_to);
            let (projected, known) =
                rrsp::new_room(inc.earned_income_cents, limit, inc.pension_adjustment_cents);
            let frac = elapsed_fraction.clamp(0.0, 1.0);
            let accrued = (projected as f64 * frac).round() as Cents;
            RrspProjection {
                income_year: current_year,
                applies_to_year: applies_to,
                income: inc.earned_income_cents,
                is_estimate: inc.is_estimate,
                projected_new_room: projected,
                accrued_to_date: accrued,
                elapsed_fraction: frac,
                dollar_limit_missing: !known,
            }
        });

    Ok(RrspSummary {
        years,
        current_room,
        total_contributed,
        current_over_contribution,
        opening_room,
        missing_limit_years,
        latest_built_in_year: rrsp::latest_known_limit_year(),
        projection,
    })
}

#[tauri::command]
fn upsert_annual_income(
    state: State<AppState>,
    person_id: i64,
    year: i32,
    earned_income_cents: Cents,
    pension_adjustment_cents: Cents,
    is_estimate: bool,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::upsert_annual_income(
        &conn,
        person_id,
        &AnnualIncome { year, earned_income_cents, pension_adjustment_cents, is_estimate },
    )
    .map_err(to_err)?;
    auto_backup(&conn, &format!("Set {year} earned income"));
    Ok(())
}

#[tauri::command]
fn list_annual_income(state: State<AppState>, person_id: i64) -> Result<Vec<AnnualIncome>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::list_annual_income(&conn, person_id).map_err(to_err)
}

#[tauri::command]
fn delete_annual_income(state: State<AppState>, person_id: i64, year: i32) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::delete_annual_income(&conn, person_id, year).map_err(to_err)?;
    auto_backup(&conn, &format!("Delete {year} earned income"));
    Ok(())
}

#[tauri::command]
fn get_rrsp_opening_room(state: State<AppState>, person_id: i64) -> Result<Cents, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::get_rrsp_opening_room(&conn, person_id).map_err(to_err)
}

#[tauri::command]
fn set_rrsp_opening_room(state: State<AppState>, person_id: i64, cents: Cents) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::set_rrsp_opening_room(&conn, person_id, cents).map_err(to_err)?;
    auto_backup(&conn, "Set RRSP opening room");
    Ok(())
}

#[tauri::command]
fn set_rrsp_dollar_limit(state: State<AppState>, year: i32, amount_cents: Cents) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::upsert_rrsp_dollar_limit(&conn, year, amount_cents).map_err(to_err)?;
    auto_backup(&conn, &format!("Set RRSP dollar limit for {year}"));
    Ok(())
}

#[tauri::command]
fn list_rrsp_dollar_limits(state: State<AppState>) -> Result<Vec<(i32, Cents)>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let overrides = db::list_rrsp_dollar_limit_overrides(&conn).map_err(to_err)?;
    Ok(overrides.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Contributions & withdrawals (shared across accounts)
// ---------------------------------------------------------------------------

#[tauri::command]
fn add_contribution(
    state: State<AppState>,
    person_id: i64,
    account: String,
    tax_year: i32,
    date: String,
    amount_cents: Cents,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let id = db::add_contribution(&conn, person_id, &account, tax_year, &date, amount_cents, &note)
        .map_err(to_err)?;
    auto_backup(&conn, &format!("Add {account} contribution ({tax_year})"));
    Ok(id)
}

#[tauri::command]
fn delete_contribution(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::delete_contribution(&conn, id).map_err(to_err)?;
    auto_backup(&conn, &format!("Delete contribution #{id}"));
    Ok(())
}

#[tauri::command]
fn list_contributions(
    state: State<AppState>,
    person_id: i64,
    account: String,
) -> Result<Vec<db::Contribution>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::list_contributions(&conn, person_id, &account).map_err(to_err)
}

#[tauri::command]
fn add_withdrawal(
    state: State<AppState>,
    person_id: i64,
    account: String,
    tax_year: i32,
    date: String,
    amount_cents: Cents,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let id = db::add_withdrawal(&conn, person_id, &account, tax_year, &date, amount_cents, &note)
        .map_err(to_err)?;
    auto_backup(&conn, &format!("Add {account} withdrawal ({tax_year})"));
    Ok(id)
}

#[tauri::command]
fn delete_withdrawal(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::delete_withdrawal(&conn, id).map_err(to_err)?;
    auto_backup(&conn, &format!("Delete withdrawal #{id}"));
    Ok(())
}

#[tauri::command]
fn list_withdrawals(
    state: State<AppState>,
    person_id: i64,
    account: String,
) -> Result<Vec<db::Withdrawal>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::list_withdrawals(&conn, person_id, &account).map_err(to_err)
}

// ---------------------------------------------------------------------------
// TFSA
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct TfsaSummary {
    years: Vec<tfsa::YearComputation>,
    current_room: Cents,
    total_contributed: Cents,
    total_withdrawn: Cents,
    current_over_contribution: Cents,
    start_year: Option<i32>,
    opening_room: Cents,
    configured: bool,
}

#[tauri::command]
fn get_tfsa_summary(
    state: State<AppState>,
    person_id: i64,
    current_year: i32,
) -> Result<TfsaSummary, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let start_year = db::get_tfsa_start_year(&conn, person_id).map_err(to_err)?;
    let opening_room = db::get_tfsa_opening_room(&conn, person_id).map_err(to_err)?;
    let data = db::tfsa_year_data(&conn, person_id, current_year).map_err(to_err)?;
    let years = tfsa::compute(&data, opening_room);

    let current_room = years.last().map(|y| y.closing_room).unwrap_or(opening_room);
    let current_over_contribution = years.last().map(|y| y.over_contribution).unwrap_or(0);
    let total_contributed = years.iter().map(|y| y.contribution).sum();
    let total_withdrawn = years.iter().map(|y| y.withdrawal).sum();

    Ok(TfsaSummary {
        years,
        current_room,
        total_contributed,
        total_withdrawn,
        current_over_contribution,
        start_year,
        opening_room,
        configured: start_year.is_some(),
    })
}

#[derive(Debug, Serialize)]
struct TfsaSettings {
    start_year: Option<i32>,
    opening_room: Cents,
}

#[tauri::command]
fn get_tfsa_settings(state: State<AppState>, person_id: i64) -> Result<TfsaSettings, String> {
    let conn = state.db.lock().map_err(to_err)?;
    Ok(TfsaSettings {
        start_year: db::get_tfsa_start_year(&conn, person_id).map_err(to_err)?,
        opening_room: db::get_tfsa_opening_room(&conn, person_id).map_err(to_err)?,
    })
}

#[tauri::command]
fn set_tfsa_settings(
    state: State<AppState>,
    person_id: i64,
    start_year: i32,
    opening_room_cents: Cents,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::set_tfsa_settings(&conn, person_id, start_year, opening_room_cents).map_err(to_err)?;
    auto_backup(&conn, "Set TFSA settings");
    Ok(())
}

// ---------------------------------------------------------------------------
// FHSA
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FhsaSummary {
    years: Vec<fhsa::YearComputation>,
    current_room: Cents,
    total_contributed: Cents,
    total_withdrawn: Cents,
    lifetime_remaining: Cents,
    current_over_contribution: Cents,
    open_year: Option<i32>,
    configured: bool,
    past_window: bool,
}

#[tauri::command]
fn get_fhsa_summary(
    state: State<AppState>,
    person_id: i64,
    current_year: i32,
) -> Result<FhsaSummary, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let open_year = db::get_fhsa_open_year(&conn, person_id).map_err(to_err)?;
    let data = db::fhsa_year_data(&conn, person_id, current_year).map_err(to_err)?;
    let years = open_year.map(|oy| fhsa::compute(&data, oy)).unwrap_or_default();

    let current_room = years.last().map(|y| y.closing_room).unwrap_or(0);
    let current_over_contribution = years.last().map(|y| y.over_contribution).unwrap_or(0);
    let lifetime_remaining = years
        .last()
        .map(|y| y.lifetime_remaining)
        .unwrap_or(fhsa::LIFETIME_LIMIT);
    let total_contributed = years.iter().map(|y| y.contribution).sum();
    let total_withdrawn = years.iter().map(|y| y.withdrawal).sum();
    let past_window = years.last().map(|y| y.past_participation_window).unwrap_or(false);

    Ok(FhsaSummary {
        years,
        current_room,
        total_contributed,
        total_withdrawn,
        lifetime_remaining,
        current_over_contribution,
        open_year,
        configured: open_year.is_some(),
        past_window,
    })
}

#[derive(Debug, Serialize)]
struct FhsaSettings {
    open_year: Option<i32>,
}

#[tauri::command]
fn get_fhsa_settings(state: State<AppState>, person_id: i64) -> Result<FhsaSettings, String> {
    let conn = state.db.lock().map_err(to_err)?;
    Ok(FhsaSettings {
        open_year: db::get_fhsa_open_year(&conn, person_id).map_err(to_err)?,
    })
}

#[tauri::command]
fn set_fhsa_settings(state: State<AppState>, person_id: i64, open_year: i32) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::set_fhsa_open_year(&conn, person_id, open_year).map_err(to_err)?;
    auto_backup(&conn, "Set FHSA settings");
    Ok(())
}

// ---------------------------------------------------------------------------
// Backup (global)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct BackupSettings {
    remote: String,
    folder: String,
    dir: String,
    enabled: bool,
}

#[tauri::command]
fn get_backup_settings(state: State<AppState>) -> Result<BackupSettings, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let cfg = BackupConfig::load(&conn);
    Ok(BackupSettings {
        remote: cfg.rclone_remote.clone(),
        folder: cfg.rclone_folder.clone(),
        dir: cfg.dir.display().to_string(),
        enabled: cfg.rclone_enabled(),
    })
}

#[tauri::command]
fn set_backup_settings(state: State<AppState>, remote: String, folder: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    BackupConfig::save(&conn, remote.trim(), folder.trim()).map_err(to_err)
}

#[tauri::command]
fn backup_now(state: State<AppState>) -> Result<BackupReport, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let cfg = BackupConfig::load(&conn);
    backup::back_up(&conn, &cfg, "Manual backup")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = db::default_db_path();
            let conn = db::open(&path).expect("failed to open database");
            db::ensure_default_person(&conn).expect("failed to ensure default person");
            app.manage(AppState { db: Mutex::new(conn) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_persons,
            add_person,
            rename_person,
            delete_person,
            get_rrsp_summary,
            upsert_annual_income,
            list_annual_income,
            delete_annual_income,
            get_rrsp_opening_room,
            set_rrsp_opening_room,
            set_rrsp_dollar_limit,
            list_rrsp_dollar_limits,
            add_contribution,
            delete_contribution,
            list_contributions,
            add_withdrawal,
            delete_withdrawal,
            list_withdrawals,
            get_tfsa_summary,
            get_tfsa_settings,
            set_tfsa_settings,
            get_fhsa_summary,
            get_fhsa_settings,
            set_fhsa_settings,
            get_backup_settings,
            set_backup_settings,
            backup_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! CRAcked — Tauri application entry point and command surface.
//!
//! The Rust side owns the SQLite database ([`db`]) and the contribution-room
//! rule engines ([`rrsp`]). The web frontend calls the `#[tauri::command]`
//! functions below via `invoke(...)`.

mod db;
mod rrsp;

use db::AnnualIncome;
use rrsp::{Cents, YearComputation};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};

/// Shared application state: the open database connection behind a mutex.
struct AppState {
    db: Mutex<rusqlite::Connection>,
}

/// Convert any error into a string for the frontend (Tauri command errors must
/// be serializable).
fn to_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// The complete RRSP picture returned to the UI.
#[derive(Debug, Serialize)]
struct RrspSummary {
    /// Per-year breakdown from the rule engine.
    years: Vec<YearComputation>,
    /// Unused room available going forward (closing room of the latest year),
    /// in cents. This is the headline "room you have left" number.
    current_room: Cents,
    /// Total contributed across all tracked years, in cents.
    total_contributed: Cents,
    /// Cumulative over-contribution beyond the buffer right now, in cents.
    current_over_contribution: Cents,
    /// Opening room carried in before the earliest tracked year, in cents.
    opening_room: Cents,
}

#[tauri::command]
fn get_rrsp_summary(state: State<AppState>) -> Result<RrspSummary, String> {
    let conn = state.db.lock().map_err(to_err)?;
    let opening_room = db::get_rrsp_opening_room(&conn).map_err(to_err)?;
    let data = db::rrsp_year_data(&conn).map_err(to_err)?;
    let years = rrsp::compute(&data, opening_room);

    let current_room = years.last().map(|y| y.closing_room).unwrap_or(opening_room);
    let current_over_contribution = years.last().map(|y| y.over_contribution).unwrap_or(0);
    let total_contributed = years.iter().map(|y| y.contribution).sum();

    Ok(RrspSummary {
        years,
        current_room,
        total_contributed,
        current_over_contribution,
        opening_room,
    })
}

#[tauri::command]
fn add_contribution(
    state: State<AppState>,
    account: String,
    tax_year: i32,
    date: String,
    amount_cents: Cents,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::add_contribution(&conn, &account, tax_year, &date, amount_cents, &note).map_err(to_err)
}

#[tauri::command]
fn delete_contribution(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::delete_contribution(&conn, id).map_err(to_err)
}

#[tauri::command]
fn list_contributions(
    state: State<AppState>,
    account: String,
) -> Result<Vec<db::Contribution>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::list_contributions(&conn, &account).map_err(to_err)
}

#[tauri::command]
fn upsert_annual_income(
    state: State<AppState>,
    year: i32,
    earned_income_cents: Cents,
    pension_adjustment_cents: Cents,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::upsert_annual_income(
        &conn,
        &AnnualIncome {
            year,
            earned_income_cents,
            pension_adjustment_cents,
        },
    )
    .map_err(to_err)
}

#[tauri::command]
fn list_annual_income(state: State<AppState>) -> Result<Vec<AnnualIncome>, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::list_annual_income(&conn).map_err(to_err)
}

#[tauri::command]
fn get_rrsp_opening_room(state: State<AppState>) -> Result<Cents, String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::get_rrsp_opening_room(&conn).map_err(to_err)
}

#[tauri::command]
fn set_rrsp_opening_room(state: State<AppState>, cents: Cents) -> Result<(), String> {
    let conn = state.db.lock().map_err(to_err)?;
    db::set_rrsp_opening_room(&conn, cents).map_err(to_err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = db::default_db_path();
            let conn = db::open(&path).expect("failed to open database");
            app.manage(AppState { db: Mutex::new(conn) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_rrsp_summary,
            add_contribution,
            delete_contribution,
            list_contributions,
            upsert_annual_income,
            list_annual_income,
            get_rrsp_opening_room,
            set_rrsp_opening_room,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

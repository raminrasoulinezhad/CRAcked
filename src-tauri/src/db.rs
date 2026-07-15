//! SQLite persistence layer for CRAcked.
//!
//! One SQLite file holds everything. Money is stored as integer **cents**
//! (see [`crate::rrsp::Cents`]).
//!
//! ## Multi-person (family) model
//!
//! CRAcked tracks a whole family. A [`Person`] owns their own income,
//! contributions, withdrawals and per-account settings — so every data call is
//! scoped by `person_id`. Two things are **global** (shared across everyone):
//! the RRSP annual dollar limits (a CRA fact) and the backup configuration.
//!
//! Timing note for RRSP: earned income reported *for* a calendar year drives the
//! *following* year's room, so [`rrsp_year_data`] reads year `Y-1` for year `Y`.

use crate::rrsp::Cents;
use crate::{fhsa, rrsp, tfsa};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// Where the database lives: `<data-dir>/CRAcked/cracked.db`.
pub fn default_db_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("CRAcked").join("cracked.db")
}

/// Open (creating if needed) the database at `path` and apply the schema.
pub fn open(path: &PathBuf) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

/// Open an in-memory database (used by tests).
#[cfg(test)]
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Schema & migration
// ---------------------------------------------------------------------------

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS person (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        name       TEXT    NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0
    );

    -- Global key/value (backup config, etc.). NOT per-person.
    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    -- Per-person key/value (RRSP opening room, TFSA start year, FHSA open year…).
    CREATE TABLE IF NOT EXISTS person_setting (
        person_id INTEGER NOT NULL,
        key       TEXT    NOT NULL,
        value     TEXT    NOT NULL,
        PRIMARY KEY (person_id, key)
    );

    CREATE TABLE IF NOT EXISTS annual_income (
        person_id                INTEGER NOT NULL,
        year                     INTEGER NOT NULL,
        earned_income_cents      INTEGER NOT NULL DEFAULT 0,
        pension_adjustment_cents INTEGER NOT NULL DEFAULT 0,
        is_estimate              INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (person_id, year)
    );

    CREATE TABLE IF NOT EXISTS contribution (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        person_id    INTEGER NOT NULL,
        account      TEXT    NOT NULL,           -- 'RRSP' | 'TFSA' | 'FHSA'
        tax_year     INTEGER NOT NULL,
        date         TEXT    NOT NULL,
        amount_cents INTEGER NOT NULL,
        note         TEXT    NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_contribution_person_account_year
        ON contribution(person_id, account, tax_year);

    CREATE TABLE IF NOT EXISTS withdrawal (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        person_id    INTEGER NOT NULL,
        account      TEXT    NOT NULL,
        tax_year     INTEGER NOT NULL,
        date         TEXT    NOT NULL,
        amount_cents INTEGER NOT NULL,
        note         TEXT    NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_withdrawal_person_account_year
        ON withdrawal(person_id, account, tax_year);

    -- Global: CRA RRSP annual dollar limits (shared by everyone).
    CREATE TABLE IF NOT EXISTS rrsp_dollar_limit (
        year         INTEGER PRIMARY KEY,
        amount_cents INTEGER NOT NULL
    );
"#;

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn migrate(conn: &Connection) -> Result<()> {
    // Migrate a pre-multi-person (v1) database created before this schema.
    // Detect it: contribution exists but has no person_id column.
    let needs_v2 = table_exists(conn, "contribution")?
        && !column_exists(conn, "contribution", "person_id")?;

    conn.execute_batch(SCHEMA)?;

    if needs_v2 {
        migrate_v1_to_v2(conn)?;
    }
    // v3 -> v4: estimated current-year income flag.
    if !column_exists(conn, "annual_income", "is_estimate")? {
        conn.execute_batch(
            "ALTER TABLE annual_income ADD COLUMN is_estimate INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    ensure_default_person(conn)?;
    Ok(())
}

/// Attach a `person_id` (defaulting existing rows to the first person) to the
/// old single-person tables, and move per-person settings across.
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    // Ensure a person to attribute existing data to.
    ensure_default_person(conn)?;

    if !column_exists(conn, "contribution", "person_id")? {
        conn.execute_batch(
            "ALTER TABLE contribution ADD COLUMN person_id INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if table_exists(conn, "withdrawal")? && !column_exists(conn, "withdrawal", "person_id")? {
        conn.execute_batch(
            "ALTER TABLE withdrawal ADD COLUMN person_id INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    // annual_income needs a composite PK, so rebuild it.
    if table_exists(conn, "annual_income")? && !column_exists(conn, "annual_income", "person_id")? {
        conn.execute_batch(
            r#"
            CREATE TABLE annual_income_v2 (
                person_id                INTEGER NOT NULL,
                year                     INTEGER NOT NULL,
                earned_income_cents      INTEGER NOT NULL DEFAULT 0,
                pension_adjustment_cents INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (person_id, year)
            );
            INSERT INTO annual_income_v2(person_id, year, earned_income_cents, pension_adjustment_cents)
                SELECT 1, year, earned_income_cents, pension_adjustment_cents FROM annual_income;
            DROP TABLE annual_income;
            ALTER TABLE annual_income_v2 RENAME TO annual_income;
            "#,
        )?;
    }
    // Move per-person settings out of the global settings table to person 1.
    for key in [
        "rrsp_opening_room_cents",
        "tfsa_start_year",
        "tfsa_opening_room_cents",
        "fhsa_open_year",
    ] {
        if let Some(val) = get_setting(conn, key)? {
            conn.execute(
                "INSERT OR REPLACE INTO person_setting(person_id, key, value) VALUES(1, ?1, ?2)",
                params![key, val],
            )?;
            conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Person {
    pub id: i64,
    pub name: String,
}

/// Ensure at least one person exists; returns the id of the first person.
pub fn ensure_default_person(conn: &Connection) -> Result<i64> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM person", [], |r| r.get(0))?;
    if count == 0 {
        conn.execute(
            "INSERT INTO person(name, sort_order) VALUES('Me', 0)",
            [],
        )?;
    }
    conn.query_row("SELECT id FROM person ORDER BY sort_order, id LIMIT 1", [], |r| r.get(0))
}

pub fn list_persons(conn: &Connection) -> Result<Vec<Person>> {
    let mut stmt = conn.prepare("SELECT id, name FROM person ORDER BY sort_order, id")?;
    let rows = stmt.query_map([], |row| {
        Ok(Person { id: row.get(0)?, name: row.get(1)? })
    })?;
    rows.collect()
}

pub fn add_person(conn: &Connection, name: &str) -> Result<i64> {
    let next_order: i64 = conn
        .query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM person", [], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO person(name, sort_order) VALUES(?1, ?2)",
        params![name, next_order],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn rename_person(conn: &Connection, id: i64, name: &str) -> Result<()> {
    conn.execute("UPDATE person SET name = ?1 WHERE id = ?2", params![name, id])?;
    Ok(())
}

/// Delete a person and all of their data.
pub fn delete_person(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM contribution WHERE person_id = ?1", params![id])?;
    conn.execute("DELETE FROM withdrawal WHERE person_id = ?1", params![id])?;
    conn.execute("DELETE FROM annual_income WHERE person_id = ?1", params![id])?;
    conn.execute("DELETE FROM person_setting WHERE person_id = ?1", params![id])?;
    conn.execute("DELETE FROM person WHERE id = ?1", params![id])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Global & per-person settings
// ---------------------------------------------------------------------------

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| {
        row.get::<_, String>(0)
    })
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn get_person_setting(conn: &Connection, person_id: i64, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM person_setting WHERE person_id = ?1 AND key = ?2",
        params![person_id, key],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

fn set_person_setting(conn: &Connection, person_id: i64, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO person_setting(person_id, key, value) VALUES(?1, ?2, ?3)
         ON CONFLICT(person_id, key) DO UPDATE SET value = excluded.value",
        params![person_id, key, value],
    )?;
    Ok(())
}

fn get_person_setting_i64(conn: &Connection, person_id: i64, key: &str) -> Result<Option<i64>> {
    Ok(get_person_setting(conn, person_id, key)?.and_then(|s| s.parse::<i64>().ok()))
}

const KEY_RRSP_OPENING_ROOM: &str = "rrsp_opening_room_cents";
const KEY_TFSA_START_YEAR: &str = "tfsa_start_year";
const KEY_TFSA_OPENING_ROOM: &str = "tfsa_opening_room_cents";
const KEY_FHSA_OPEN_YEAR: &str = "fhsa_open_year";

pub fn get_rrsp_opening_room(conn: &Connection, person_id: i64) -> Result<Cents> {
    Ok(get_person_setting_i64(conn, person_id, KEY_RRSP_OPENING_ROOM)?.unwrap_or(0))
}

pub fn set_rrsp_opening_room(conn: &Connection, person_id: i64, cents: Cents) -> Result<()> {
    set_person_setting(conn, person_id, KEY_RRSP_OPENING_ROOM, &cents.to_string())
}

pub fn get_tfsa_start_year(conn: &Connection, person_id: i64) -> Result<Option<i32>> {
    Ok(get_person_setting_i64(conn, person_id, KEY_TFSA_START_YEAR)?.map(|v| v as i32))
}

pub fn get_tfsa_opening_room(conn: &Connection, person_id: i64) -> Result<Cents> {
    Ok(get_person_setting_i64(conn, person_id, KEY_TFSA_OPENING_ROOM)?.unwrap_or(0))
}

pub fn set_tfsa_settings(
    conn: &Connection,
    person_id: i64,
    start_year: i32,
    opening_room: Cents,
) -> Result<()> {
    set_person_setting(conn, person_id, KEY_TFSA_START_YEAR, &start_year.to_string())?;
    set_person_setting(conn, person_id, KEY_TFSA_OPENING_ROOM, &opening_room.to_string())
}

pub fn get_fhsa_open_year(conn: &Connection, person_id: i64) -> Result<Option<i32>> {
    Ok(get_person_setting_i64(conn, person_id, KEY_FHSA_OPEN_YEAR)?.map(|v| v as i32))
}

pub fn set_fhsa_open_year(conn: &Connection, person_id: i64, open_year: i32) -> Result<()> {
    set_person_setting(conn, person_id, KEY_FHSA_OPEN_YEAR, &open_year.to_string())
}

// --- RRSP annual dollar-limit overrides (GLOBAL) ----------------------------

pub fn upsert_rrsp_dollar_limit(conn: &Connection, year: i32, amount_cents: Cents) -> Result<()> {
    conn.execute(
        "INSERT INTO rrsp_dollar_limit(year, amount_cents) VALUES(?1, ?2)
         ON CONFLICT(year) DO UPDATE SET amount_cents = excluded.amount_cents",
        params![year, amount_cents],
    )?;
    Ok(())
}

pub fn list_rrsp_dollar_limit_overrides(conn: &Connection) -> Result<BTreeMap<i32, Cents>> {
    let mut stmt =
        conn.prepare("SELECT year, amount_cents FROM rrsp_dollar_limit ORDER BY year")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i32>(0)?, row.get::<_, Cents>(1)?)))?;
    let mut map = BTreeMap::new();
    for r in rows {
        let (y, c) = r?;
        map.insert(y, c);
    }
    Ok(map)
}

/// Resolve the effective RRSP dollar limit for a year: user override wins, then
/// the built-in table, else `None`.
pub fn resolve_rrsp_limit(overrides: &BTreeMap<i32, Cents>, year: i32) -> Option<Cents> {
    overrides.get(&year).copied().or_else(|| rrsp::annual_dollar_limit(year))
}

// ---------------------------------------------------------------------------
// Annual income (per person)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnnualIncome {
    pub year: i32,
    pub earned_income_cents: Cents,
    pub pension_adjustment_cents: Cents,
    /// True when this is an estimate for a not-yet-final (current) year.
    #[serde(default)]
    pub is_estimate: bool,
}

pub fn upsert_annual_income(conn: &Connection, person_id: i64, income: &AnnualIncome) -> Result<()> {
    conn.execute(
        "INSERT INTO annual_income(person_id, year, earned_income_cents, pension_adjustment_cents, is_estimate)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(person_id, year) DO UPDATE SET
             earned_income_cents      = excluded.earned_income_cents,
             pension_adjustment_cents = excluded.pension_adjustment_cents,
             is_estimate              = excluded.is_estimate",
        params![
            person_id,
            income.year,
            income.earned_income_cents,
            income.pension_adjustment_cents,
            income.is_estimate as i64
        ],
    )?;
    Ok(())
}

/// Look up one person's income record for a specific year.
pub fn get_annual_income(conn: &Connection, person_id: i64, year: i32) -> Result<Option<AnnualIncome>> {
    conn.query_row(
        "SELECT year, earned_income_cents, pension_adjustment_cents, is_estimate
         FROM annual_income WHERE person_id = ?1 AND year = ?2",
        params![person_id, year],
        |row| {
            Ok(AnnualIncome {
                year: row.get(0)?,
                earned_income_cents: row.get(1)?,
                pension_adjustment_cents: row.get(2)?,
                is_estimate: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

pub fn delete_annual_income(conn: &Connection, person_id: i64, year: i32) -> Result<()> {
    conn.execute(
        "DELETE FROM annual_income WHERE person_id = ?1 AND year = ?2",
        params![person_id, year],
    )?;
    Ok(())
}

pub fn list_annual_income(conn: &Connection, person_id: i64) -> Result<Vec<AnnualIncome>> {
    let mut stmt = conn.prepare(
        "SELECT year, earned_income_cents, pension_adjustment_cents, is_estimate
         FROM annual_income WHERE person_id = ?1 ORDER BY year",
    )?;
    let rows = stmt.query_map(params![person_id], |row| {
        Ok(AnnualIncome {
            year: row.get(0)?,
            earned_income_cents: row.get(1)?,
            pension_adjustment_cents: row.get(2)?,
            is_estimate: row.get::<_, i64>(3)? != 0,
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Contributions (per person, per account)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub id: i64,
    pub person_id: i64,
    pub account: String,
    pub tax_year: i32,
    pub date: String,
    pub amount_cents: Cents,
    pub note: String,
}

pub fn add_contribution(
    conn: &Connection,
    person_id: i64,
    account: &str,
    tax_year: i32,
    date: &str,
    amount_cents: Cents,
    note: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO contribution(person_id, account, tax_year, date, amount_cents, note)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![person_id, account, tax_year, date, amount_cents, note],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_contribution(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM contribution WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_contributions(
    conn: &Connection,
    person_id: i64,
    account: &str,
) -> Result<Vec<Contribution>> {
    let mut stmt = conn.prepare(
        "SELECT id, person_id, account, tax_year, date, amount_cents, note
         FROM contribution WHERE person_id = ?1 AND account = ?2
         ORDER BY tax_year, date, id",
    )?;
    let rows = stmt.query_map(params![person_id, account], |row| {
        Ok(Contribution {
            id: row.get(0)?,
            person_id: row.get(1)?,
            account: row.get(2)?,
            tax_year: row.get(3)?,
            date: row.get(4)?,
            amount_cents: row.get(5)?,
            note: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn contributions_by_year(
    conn: &Connection,
    person_id: i64,
    account: &str,
) -> Result<BTreeMap<i32, Cents>> {
    let mut stmt = conn.prepare(
        "SELECT tax_year, COALESCE(SUM(amount_cents), 0)
         FROM contribution WHERE person_id = ?1 AND account = ?2 GROUP BY tax_year",
    )?;
    let rows = stmt.query_map(params![person_id, account], |row| {
        Ok((row.get::<_, i32>(0)?, row.get::<_, Cents>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for r in rows {
        let (y, c) = r?;
        map.insert(y, c);
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Withdrawals (per person, per account)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Withdrawal {
    pub id: i64,
    pub person_id: i64,
    pub account: String,
    pub tax_year: i32,
    pub date: String,
    pub amount_cents: Cents,
    pub note: String,
}

pub fn add_withdrawal(
    conn: &Connection,
    person_id: i64,
    account: &str,
    tax_year: i32,
    date: &str,
    amount_cents: Cents,
    note: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO withdrawal(person_id, account, tax_year, date, amount_cents, note)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![person_id, account, tax_year, date, amount_cents, note],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_withdrawal(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM withdrawal WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_withdrawals(conn: &Connection, person_id: i64, account: &str) -> Result<Vec<Withdrawal>> {
    let mut stmt = conn.prepare(
        "SELECT id, person_id, account, tax_year, date, amount_cents, note
         FROM withdrawal WHERE person_id = ?1 AND account = ?2
         ORDER BY tax_year, date, id",
    )?;
    let rows = stmt.query_map(params![person_id, account], |row| {
        Ok(Withdrawal {
            id: row.get(0)?,
            person_id: row.get(1)?,
            account: row.get(2)?,
            tax_year: row.get(3)?,
            date: row.get(4)?,
            amount_cents: row.get(5)?,
            note: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn withdrawals_by_year(
    conn: &Connection,
    person_id: i64,
    account: &str,
) -> Result<BTreeMap<i32, Cents>> {
    let mut stmt = conn.prepare(
        "SELECT tax_year, COALESCE(SUM(amount_cents), 0)
         FROM withdrawal WHERE person_id = ?1 AND account = ?2 GROUP BY tax_year",
    )?;
    let rows = stmt.query_map(params![person_id, account], |row| {
        Ok((row.get::<_, i32>(0)?, row.get::<_, Cents>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for r in rows {
        let (y, c) = r?;
        map.insert(y, c);
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Per-account aggregation (bridge to the rule engines), scoped by person
// ---------------------------------------------------------------------------

pub fn rrsp_year_data(conn: &Connection, person_id: i64) -> Result<Vec<rrsp::YearData>> {
    let incomes = list_annual_income(conn, person_id)?;
    let contribs = contributions_by_year(conn, person_id, "RRSP")?;
    let overrides = list_rrsp_dollar_limit_overrides(conn)?;

    let income_by_year: BTreeMap<i32, &AnnualIncome> =
        incomes.iter().map(|i| (i.year, i)).collect();

    let candidates = income_by_year.keys().map(|y| y + 1).chain(contribs.keys().copied());
    let (min_y, max_y) = candidates.fold((i32::MAX, i32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
    if min_y == i32::MAX {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for year in min_y..=max_y {
        let prior = income_by_year.get(&(year - 1));
        out.push(rrsp::YearData {
            year,
            prior_year_earned_income: prior.map(|i| i.earned_income_cents).unwrap_or(0),
            pension_adjustment: prior.map(|i| i.pension_adjustment_cents).unwrap_or(0),
            contribution: contribs.get(&year).copied().unwrap_or(0),
            dollar_limit: resolve_rrsp_limit(&overrides, year),
        });
    }
    Ok(out)
}

pub fn tfsa_year_data(
    conn: &Connection,
    person_id: i64,
    current_year: i32,
) -> Result<Vec<tfsa::YearData>> {
    let start = match get_tfsa_start_year(conn, person_id)? {
        Some(s) => s,
        None => return Ok(Vec::new()),
    };
    let contribs = contributions_by_year(conn, person_id, "TFSA")?;
    let withdrawals = withdrawals_by_year(conn, person_id, "TFSA")?;

    let max_txn = contribs.keys().chain(withdrawals.keys()).copied().max().unwrap_or(start);
    let end = current_year.max(max_txn).max(start);

    let mut out = Vec::new();
    for year in start..=end {
        out.push(tfsa::YearData {
            year,
            eligible: year >= start,
            contribution: contribs.get(&year).copied().unwrap_or(0),
            withdrawal: withdrawals.get(&year).copied().unwrap_or(0),
        });
    }
    Ok(out)
}

pub fn fhsa_year_data(
    conn: &Connection,
    person_id: i64,
    current_year: i32,
) -> Result<Vec<fhsa::YearData>> {
    let open_year = match get_fhsa_open_year(conn, person_id)? {
        Some(y) => y,
        None => return Ok(Vec::new()),
    };
    let contribs = contributions_by_year(conn, person_id, "FHSA")?;
    let withdrawals = withdrawals_by_year(conn, person_id, "FHSA")?;

    let max_txn = contribs.keys().chain(withdrawals.keys()).copied().max().unwrap_or(open_year);
    let end = current_year.max(max_txn).max(open_year);

    let mut out = Vec::new();
    for year in open_year..=end {
        out.push(fhsa::YearData {
            year,
            open: year >= open_year,
            contribution: contribs.get(&year).copied().unwrap_or(0),
            withdrawal: withdrawals.get(&year).copied().unwrap_or(0),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Snapshot export — the plain-text form committed to the data git repo
// ---------------------------------------------------------------------------

/// Deterministic, human-readable JSON snapshot of ALL data across all people.
pub fn export_json(conn: &Connection) -> Result<serde_json::Value> {
    let persons = list_persons(conn)?;

    let mut settings = serde_json::Map::new();
    {
        let mut stmt = conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (k, v) = r?;
            settings.insert(k, serde_json::Value::String(v));
        }
    }

    // person_setting rows, ordered.
    let mut person_settings: Vec<serde_json::Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT person_id, key, value FROM person_setting ORDER BY person_id, key",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "person_id": row.get::<_, i64>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
            }))
        })?;
        for r in rows {
            person_settings.push(r?);
        }
    }

    let mut annual_income: Vec<serde_json::Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT person_id, year, earned_income_cents, pension_adjustment_cents, is_estimate
             FROM annual_income ORDER BY person_id, year",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "person_id": row.get::<_, i64>(0)?,
                "year": row.get::<_, i32>(1)?,
                "earned_income_cents": row.get::<_, Cents>(2)?,
                "pension_adjustment_cents": row.get::<_, Cents>(3)?,
                "is_estimate": row.get::<_, i64>(4)? != 0,
            }))
        })?;
        for r in rows {
            annual_income.push(r?);
        }
    }

    let mut contributions: Vec<Contribution> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, person_id, account, tax_year, date, amount_cents, note
             FROM contribution ORDER BY person_id, account, tax_year, date, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Contribution {
                id: row.get(0)?,
                person_id: row.get(1)?,
                account: row.get(2)?,
                tax_year: row.get(3)?,
                date: row.get(4)?,
                amount_cents: row.get(5)?,
                note: row.get(6)?,
            })
        })?;
        for r in rows {
            contributions.push(r?);
        }
    }

    let mut withdrawals: Vec<Withdrawal> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, person_id, account, tax_year, date, amount_cents, note
             FROM withdrawal ORDER BY person_id, account, tax_year, date, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Withdrawal {
                id: row.get(0)?,
                person_id: row.get(1)?,
                account: row.get(2)?,
                tax_year: row.get(3)?,
                date: row.get(4)?,
                amount_cents: row.get(5)?,
                note: row.get(6)?,
            })
        })?;
        for r in rows {
            withdrawals.push(r?);
        }
    }

    let rrsp_limits: BTreeMap<i32, Cents> = list_rrsp_dollar_limit_overrides(conn)?;

    Ok(serde_json::json!({
        "schema_version": 3,
        "persons": persons,
        "settings": settings,
        "person_settings": person_settings,
        "annual_income": annual_income,
        "contributions": contributions,
        "withdrawals": withdrawals,
        "rrsp_dollar_limit_overrides": rrsp_limits,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default person's id in a fresh DB.
    fn p(conn: &Connection) -> i64 {
        ensure_default_person(conn).unwrap()
    }

    #[test]
    fn default_person_created() {
        let conn = open_in_memory().unwrap();
        let people = list_persons(&conn).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "Me");
    }

    #[test]
    fn people_add_rename_delete_isolates_data() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        let spouse = add_person(&conn, "Alex").unwrap();
        assert_eq!(list_persons(&conn).unwrap().len(), 2);

        add_contribution(&conn, me, "RRSP", 2024, "2024-01-01", 1_000_00, "").unwrap();
        add_contribution(&conn, spouse, "RRSP", 2024, "2024-01-01", 5_000_00, "").unwrap();
        assert_eq!(list_contributions(&conn, me, "RRSP").unwrap().len(), 1);
        assert_eq!(list_contributions(&conn, spouse, "RRSP").unwrap()[0].amount_cents, 5_000_00);

        rename_person(&conn, spouse, "Alexandra").unwrap();
        assert_eq!(list_persons(&conn).unwrap()[1].name, "Alexandra");

        delete_person(&conn, spouse).unwrap();
        assert_eq!(list_persons(&conn).unwrap().len(), 1);
        assert!(list_contributions(&conn, spouse, "RRSP").unwrap().is_empty());
        // The other person's data is untouched.
        assert_eq!(list_contributions(&conn, me, "RRSP").unwrap().len(), 1);
    }

    #[test]
    fn settings_are_per_person() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        let spouse = add_person(&conn, "Alex").unwrap();
        set_tfsa_settings(&conn, me, 2015, 0).unwrap();
        set_tfsa_settings(&conn, spouse, 2020, 1_000_00).unwrap();
        assert_eq!(get_tfsa_start_year(&conn, me).unwrap(), Some(2015));
        assert_eq!(get_tfsa_start_year(&conn, spouse).unwrap(), Some(2020));
        assert_eq!(get_tfsa_opening_room(&conn, spouse).unwrap(), 1_000_00);
    }

    #[test]
    fn annual_income_upsert_and_delete() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        upsert_annual_income(&conn, me, &AnnualIncome { year: 2023, earned_income_cents: 55_000_00, pension_adjustment_cents: 0, is_estimate: false }).unwrap();
        assert_eq!(list_annual_income(&conn, me).unwrap().len(), 1);
        delete_annual_income(&conn, me, 2023).unwrap();
        assert!(list_annual_income(&conn, me).unwrap().is_empty());
    }

    #[test]
    fn rrsp_limit_override_takes_precedence() {
        let conn = open_in_memory().unwrap();
        let overrides = list_rrsp_dollar_limit_overrides(&conn).unwrap();
        assert_eq!(resolve_rrsp_limit(&overrides, 2024), Some(31_560_00));
        assert_eq!(resolve_rrsp_limit(&overrides, 2099), None);
        upsert_rrsp_dollar_limit(&conn, 2099, 40_000_00).unwrap();
        let overrides = list_rrsp_dollar_limit_overrides(&conn).unwrap();
        assert_eq!(resolve_rrsp_limit(&overrides, 2099), Some(40_000_00));
    }

    #[test]
    fn withdrawals_add_sum_delete() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        let id1 = add_withdrawal(&conn, me, "TFSA", 2024, "2024-05-01", 3_000_00, "").unwrap();
        add_withdrawal(&conn, me, "TFSA", 2024, "2024-08-01", 1_000_00, "car").unwrap();
        assert_eq!(withdrawals_by_year(&conn, me, "TFSA").unwrap().get(&2024).copied(), Some(4_000_00));
        delete_withdrawal(&conn, id1).unwrap();
        assert_eq!(withdrawals_by_year(&conn, me, "TFSA").unwrap().get(&2024).copied(), Some(1_000_00));
    }

    #[test]
    fn rrsp_year_data_maps_prior_year_income_and_limit() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        upsert_annual_income(&conn, me, &AnnualIncome { year: 2023, earned_income_cents: 50_000_00, pension_adjustment_cents: 0, is_estimate: false }).unwrap();
        add_contribution(&conn, me, "RRSP", 2024, "2024-02-01", 3_000_00, "").unwrap();
        let data = rrsp_year_data(&conn, me).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].year, 2024);
        assert_eq!(data[0].prior_year_earned_income, 50_000_00);
        assert_eq!(data[0].dollar_limit, Some(31_560_00));
    }

    #[test]
    fn tfsa_and_fhsa_year_data_span_to_current_year() {
        let conn = open_in_memory().unwrap();
        let me = p(&conn);
        assert!(tfsa_year_data(&conn, me, 2026).unwrap().is_empty());
        set_tfsa_settings(&conn, me, 2020, 0).unwrap();
        let t = tfsa_year_data(&conn, me, 2026).unwrap();
        assert_eq!(t.first().unwrap().year, 2020);
        assert_eq!(t.last().unwrap().year, 2026);

        assert!(fhsa_year_data(&conn, me, 2026).unwrap().is_empty());
        set_fhsa_open_year(&conn, me, 2023).unwrap();
        let f = fhsa_year_data(&conn, me, 2026).unwrap();
        assert_eq!(f.first().unwrap().year, 2023);
        assert!(f.iter().all(|d| d.open));
    }
}

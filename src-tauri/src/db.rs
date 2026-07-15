//! SQLite persistence layer for CRAcked.
//!
//! One SQLite file holds everything. Money is stored as integer **cents**
//! (see [`crate::rrsp::Cents`]). The schema is intentionally account-aware from
//! day one (`account` column) so TFSA and FHSA slot in later without migration.
//!
//! Timing note for RRSP: the earned income and pension adjustment reported *for*
//! a calendar year drive the *following* year's new room. We therefore store
//! them keyed by the year they were earned/reported, and the RRSP aggregation
//! ([`rrsp_year_data`]) reads year `Y-1` to build room for year `Y`.

use crate::rrsp::{Cents, YearData};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// Where the database lives: `<data-dir>/CRAcked/cracked.db`.
/// On Linux that's `~/.local/share/CRAcked/cracked.db`.
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

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Earned income & pension adjustment reported FOR a calendar year.
        -- Drives the FOLLOWING year's RRSP room.
        CREATE TABLE IF NOT EXISTS annual_income (
            year                     INTEGER PRIMARY KEY,
            earned_income_cents      INTEGER NOT NULL DEFAULT 0,
            pension_adjustment_cents INTEGER NOT NULL DEFAULT 0
        );

        -- Individual contributions, per account, tagged with the tax year the
        -- contribution counts toward.
        CREATE TABLE IF NOT EXISTS contribution (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            account      TEXT    NOT NULL,           -- 'RRSP' | 'TFSA' | 'FHSA'
            tax_year     INTEGER NOT NULL,
            date         TEXT    NOT NULL,           -- ISO 8601 'YYYY-MM-DD'
            amount_cents INTEGER NOT NULL,
            note         TEXT    NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_contribution_account_year
            ON contribution(account, tax_year);
        "#,
    )
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
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

/// Unused RRSP room carried in before the earliest tracked year, in cents.
/// Stored as a setting; defaults to 0.
const KEY_RRSP_OPENING_ROOM: &str = "rrsp_opening_room_cents";

pub fn get_rrsp_opening_room(conn: &Connection) -> Result<Cents> {
    Ok(get_setting(conn, KEY_RRSP_OPENING_ROOM)?
        .and_then(|s| s.parse::<Cents>().ok())
        .unwrap_or(0))
}

pub fn set_rrsp_opening_room(conn: &Connection, cents: Cents) -> Result<()> {
    set_setting(conn, KEY_RRSP_OPENING_ROOM, &cents.to_string())
}

// ---------------------------------------------------------------------------
// Annual income
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnnualIncome {
    pub year: i32,
    pub earned_income_cents: Cents,
    pub pension_adjustment_cents: Cents,
}

pub fn upsert_annual_income(conn: &Connection, income: &AnnualIncome) -> Result<()> {
    conn.execute(
        "INSERT INTO annual_income(year, earned_income_cents, pension_adjustment_cents)
         VALUES(?1, ?2, ?3)
         ON CONFLICT(year) DO UPDATE SET
             earned_income_cents      = excluded.earned_income_cents,
             pension_adjustment_cents = excluded.pension_adjustment_cents",
        params![
            income.year,
            income.earned_income_cents,
            income.pension_adjustment_cents
        ],
    )?;
    Ok(())
}

pub fn list_annual_income(conn: &Connection) -> Result<Vec<AnnualIncome>> {
    let mut stmt = conn.prepare(
        "SELECT year, earned_income_cents, pension_adjustment_cents
         FROM annual_income ORDER BY year",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AnnualIncome {
            year: row.get(0)?,
            earned_income_cents: row.get(1)?,
            pension_adjustment_cents: row.get(2)?,
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Contributions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub id: i64,
    pub account: String,
    pub tax_year: i32,
    pub date: String,
    pub amount_cents: Cents,
    pub note: String,
}

/// Add a contribution and return its new id.
pub fn add_contribution(
    conn: &Connection,
    account: &str,
    tax_year: i32,
    date: &str,
    amount_cents: Cents,
    note: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO contribution(account, tax_year, date, amount_cents, note)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![account, tax_year, date, amount_cents, note],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_contribution(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM contribution WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_contributions(conn: &Connection, account: &str) -> Result<Vec<Contribution>> {
    let mut stmt = conn.prepare(
        "SELECT id, account, tax_year, date, amount_cents, note
         FROM contribution WHERE account = ?1
         ORDER BY tax_year, date, id",
    )?;
    let rows = stmt.query_map(params![account], |row| {
        Ok(Contribution {
            id: row.get(0)?,
            account: row.get(1)?,
            tax_year: row.get(2)?,
            date: row.get(3)?,
            amount_cents: row.get(4)?,
            note: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Sum of contributions per tax year for one account.
pub fn contributions_by_year(conn: &Connection, account: &str) -> Result<BTreeMap<i32, Cents>> {
    let mut stmt = conn.prepare(
        "SELECT tax_year, COALESCE(SUM(amount_cents), 0)
         FROM contribution WHERE account = ?1 GROUP BY tax_year",
    )?;
    let rows = stmt.query_map(params![account], |row| {
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
// RRSP aggregation — bridge from stored rows to the rule engine's inputs
// ---------------------------------------------------------------------------

/// Build the per-year inputs for the RRSP engine from stored data.
///
/// New room for year `Y` uses income reported for year `Y-1`. We cover every
/// year from (first income year + 1) through the latest of (income+1) or a
/// contribution year, so a contribution in a year with no matching income still
/// appears (with 0 new room) rather than being silently dropped.
pub fn rrsp_year_data(conn: &Connection) -> Result<Vec<YearData>> {
    let incomes = list_annual_income(conn)?;
    let contribs = contributions_by_year(conn, "RRSP")?;

    let income_by_year: BTreeMap<i32, &AnnualIncome> =
        incomes.iter().map(|i| (i.year, i)).collect();

    // Determine the span of years to report: every year that either has income
    // from the prior year (income year + 1) or has a contribution recorded.
    let candidates = income_by_year
        .keys()
        .map(|y| y + 1)
        .chain(contribs.keys().copied());
    let (min_y, max_y) = candidates.fold((i32::MAX, i32::MIN), |(lo, hi), y| {
        (lo.min(y), hi.max(y))
    });
    if min_y == i32::MAX {
        return Ok(Vec::new()); // no data at all
    }

    let mut out = Vec::new();
    for year in min_y..=max_y {
        let prior = income_by_year.get(&(year - 1));
        out.push(YearData {
            year,
            prior_year_earned_income: prior.map(|i| i.earned_income_cents).unwrap_or(0),
            pension_adjustment: prior.map(|i| i.pension_adjustment_cents).unwrap_or(0),
            contribution: contribs.get(&year).copied().unwrap_or(0),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Snapshot export — the plain-text form committed to the data git repo
// ---------------------------------------------------------------------------

/// Produce a deterministic, human-readable JSON snapshot of ALL data. Ordering
/// is stable (by key/year/id) so successive commits diff cleanly in git.
pub fn export_json(conn: &Connection) -> Result<serde_json::Value> {
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

    let incomes = list_annual_income(conn)?;

    // All contributions across every account, ordered deterministically.
    let mut contributions: Vec<Contribution> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, account, tax_year, date, amount_cents, note
             FROM contribution ORDER BY account, tax_year, date, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Contribution {
                id: row.get(0)?,
                account: row.get(1)?,
                tax_year: row.get(2)?,
                date: row.get(3)?,
                amount_cents: row.get(4)?,
                note: row.get(5)?,
            })
        })?;
        for r in rows {
            contributions.push(r?);
        }
    }

    Ok(serde_json::json!({
        "schema_version": 1,
        "settings": settings,
        "annual_income": incomes,
        "contributions": contributions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_setting(&conn, "missing").unwrap(), None);
        set_setting(&conn, "k", "v").unwrap();
        assert_eq!(get_setting(&conn, "k").unwrap(), Some("v".into()));
        set_setting(&conn, "k", "v2").unwrap();
        assert_eq!(get_setting(&conn, "k").unwrap(), Some("v2".into()));
    }

    #[test]
    fn opening_room_defaults_to_zero_then_persists() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_rrsp_opening_room(&conn).unwrap(), 0);
        set_rrsp_opening_room(&conn, 20_000_00).unwrap();
        assert_eq!(get_rrsp_opening_room(&conn).unwrap(), 20_000_00);
    }

    #[test]
    fn annual_income_upsert_and_list() {
        let conn = open_in_memory().unwrap();
        upsert_annual_income(
            &conn,
            &AnnualIncome { year: 2023, earned_income_cents: 50_000_00, pension_adjustment_cents: 0 },
        )
        .unwrap();
        upsert_annual_income(
            &conn,
            &AnnualIncome { year: 2023, earned_income_cents: 55_000_00, pension_adjustment_cents: 1_000_00 },
        )
        .unwrap();
        let list = list_annual_income(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].earned_income_cents, 55_000_00);
        assert_eq!(list[0].pension_adjustment_cents, 1_000_00);
    }

    #[test]
    fn contributions_add_sum_delete() {
        let conn = open_in_memory().unwrap();
        let id1 = add_contribution(&conn, "RRSP", 2024, "2024-03-01", 4_000_00, "").unwrap();
        add_contribution(&conn, "RRSP", 2024, "2024-06-01", 2_000_00, "bonus").unwrap();
        add_contribution(&conn, "TFSA", 2024, "2024-06-01", 1_000_00, "").unwrap();

        let by_year = contributions_by_year(&conn, "RRSP").unwrap();
        assert_eq!(by_year.get(&2024).copied(), Some(6_000_00));
        assert_eq!(list_contributions(&conn, "RRSP").unwrap().len(), 2);

        delete_contribution(&conn, id1).unwrap();
        assert_eq!(
            contributions_by_year(&conn, "RRSP").unwrap().get(&2024).copied(),
            Some(2_000_00)
        );
    }

    #[test]
    fn rrsp_year_data_maps_prior_year_income_to_room() {
        let conn = open_in_memory().unwrap();
        // Income earned in 2023 drives 2024's room.
        upsert_annual_income(
            &conn,
            &AnnualIncome { year: 2023, earned_income_cents: 50_000_00, pension_adjustment_cents: 0 },
        )
        .unwrap();
        add_contribution(&conn, "RRSP", 2024, "2024-02-01", 3_000_00, "").unwrap();

        let data = rrsp_year_data(&conn).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].year, 2024);
        assert_eq!(data[0].prior_year_earned_income, 50_000_00);
        assert_eq!(data[0].contribution, 3_000_00);
    }
}

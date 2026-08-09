use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::types::{DailyRow, TokenCounts};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryOverrideData {
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub models: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryOverridesFile {
    #[serde(default)]
    pub monthly_overrides: BTreeMap<String, HistoryOverrideData>,
}

/// Resolve the path to `history_overrides.json` in user's config directory.
pub fn get_overrides_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let p1 = home.join(".config").join("tokenusage").join("history_overrides.json");
    if p1.is_file() {
        return Some(p1);
    }
    let p2 = home.join(".config").join("tu").join("history_overrides.json");
    if p2.is_file() {
        return Some(p2);
    }
    Some(p1)
}

/// Resolve the path to SQLite `history.db`.
pub fn get_history_db_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".config").join("tokenusage");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("history.db"))
}

/// Load overrides file if it exists.
pub fn load_overrides() -> HistoryOverridesFile {
    if let Some(path) = get_overrides_path() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(overrides) = serde_json::from_str::<HistoryOverridesFile>(&content) {
                return overrides;
            }
        }
    }
    HistoryOverridesFile::default()
}

/// Apply monthly overrides to generated report rows.
pub fn apply_monthly_overrides(rows: &mut Vec<DailyRow>) {
    let overrides = load_overrides();
    if overrides.monthly_overrides.is_empty() {
        return;
    }

    for (month_key, data) in overrides.monthly_overrides {
        let existing = rows.iter_mut().find(|r| r.date == month_key);

        if let Some(row) = existing {
            if row.totals.total_tokens < data.total_tokens {
                row.totals = TokenCounts {
                    input_tokens: data.input_tokens,
                    cache_creation_input_tokens: data.cache_creation_input_tokens,
                    cache_read_input_tokens: data.cache_read_input_tokens,
                    output_tokens: data.output_tokens,
                    reasoning_output_tokens: 0,
                    total_tokens: data.total_tokens,
                    cost_usd: data.cost_usd,
                };
                for m in &data.models {
                    row.models.entry(m.clone()).or_insert(TokenCounts::default());
                }
            }
        } else {
            let mut models = BTreeMap::new();
            for m in &data.models {
                models.insert(m.clone(), TokenCounts::default());
            }
            rows.push(DailyRow {
                date: month_key,
                totals: TokenCounts {
                    input_tokens: data.input_tokens,
                    cache_creation_input_tokens: data.cache_creation_input_tokens,
                    cache_read_input_tokens: data.cache_read_input_tokens,
                    output_tokens: data.output_tokens,
                    reasoning_output_tokens: 0,
                    total_tokens: data.total_tokens,
                    cost_usd: data.cost_usd,
                },
                models,
                sources: BTreeMap::new(),
                models_by_source: BTreeMap::new(),
                activity: None,
            });
        }
    }
}

/// Initialize SQLite history.db table schema if needed.
pub fn init_history_db() -> Result<Connection> {
    let path = get_history_db_path().context("Could not locate home directory for history.db")?;
    let conn = Connection::open(path)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_history (
            date TEXT PRIMARY KEY,
            input_tokens INTEGER NOT NULL,
            cache_creation_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            cost_usd REAL NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    )?;
    Ok(conn)
}

/// Persist current report rows to history.db SQLite.
pub fn persist_report_rows(rows: &[DailyRow]) -> Result<()> {
    let Ok(conn) = init_history_db() else {
        return Ok(());
    };
    let now = chrono::Utc::now().to_rfc3339();

    for row in rows {
        let _ = conn.execute(
            "INSERT INTO daily_history (date, input_tokens, cache_creation_tokens, cache_read_tokens, output_tokens, total_tokens, cost_usd, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(date) DO UPDATE SET
                input_tokens = MAX(input_tokens, excluded.input_tokens),
                cache_creation_tokens = MAX(cache_creation_tokens, excluded.cache_creation_tokens),
                cache_read_tokens = MAX(cache_read_tokens, excluded.cache_read_tokens),
                output_tokens = MAX(output_tokens, excluded.output_tokens),
                total_tokens = MAX(total_tokens, excluded.total_tokens),
                cost_usd = MAX(cost_usd, excluded.cost_usd),
                updated_at = excluded.updated_at",
            params![
                row.date,
                row.totals.input_tokens as i64,
                row.totals.cache_creation_input_tokens as i64,
                row.totals.cache_read_input_tokens as i64,
                row.totals.output_tokens as i64,
                row.totals.total_tokens as i64,
                row.totals.cost_usd,
                now
            ],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_monthly_overrides_updates_lower_row() {
        let mut rows = vec![DailyRow {
            date: "2026-03".to_string(),
            totals: TokenCounts {
                input_tokens: 100,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: 10,
                reasoning_output_tokens: 0,
                total_tokens: 110,
                cost_usd: 1.0,
            },
            models: BTreeMap::new(),
            sources: BTreeMap::new(),
            models_by_source: BTreeMap::new(),
            activity: None,
        }];

        apply_monthly_overrides(&mut rows);

        let row_2026_03 = rows.iter().find(|r| r.date == "2026-03").unwrap();
        assert!(row_2026_03.totals.total_tokens >= 1548396561);
        assert!(row_2026_03.totals.cost_usd >= 1255.63);
    }
}


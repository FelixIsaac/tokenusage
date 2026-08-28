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
    let p1 = home
        .join(".config")
        .join("tokenusage")
        .join("history_overrides.json");
    if p1.is_file() {
        return Some(p1);
    }
    let p2 = home
        .join(".config")
        .join("tu")
        .join("history_overrides.json");
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

use crate::cli::CommonArgs;
use crate::types::SourceKind;

pub fn infer_model_source(model: &str) -> Option<SourceKind> {
    let lower = model.to_lowercase();
    if lower.contains("claude") {
        Some(SourceKind::Claude)
    } else if lower.contains("gpt")
        || lower.contains("codex")
        || lower.contains("o1")
        || lower.contains("o3")
    {
        Some(SourceKind::Codex)
    } else if lower.contains("gemini") || lower.contains("gemma") {
        Some(SourceKind::Gemini)
    } else if lower.contains("grok") {
        Some(SourceKind::Grok)
    } else if lower.contains("opencode") {
        Some(SourceKind::OpenCode)
    } else {
        None
    }
}

/// Apply monthly overrides to generated report rows.
pub fn apply_monthly_overrides(rows: &mut Vec<DailyRow>, common: &CommonArgs) {
    let overrides = load_overrides();
    apply_monthly_overrides_with_data_and_filter(rows, &overrides, common);
}

#[allow(dead_code)]
pub fn apply_monthly_overrides_with_data(
    rows: &mut Vec<DailyRow>,
    overrides: &HistoryOverridesFile,
) {
    apply_monthly_overrides_with_data_and_filter(rows, overrides, &CommonArgs::default());
}

pub fn apply_monthly_overrides_with_data_and_filter(
    rows: &mut Vec<DailyRow>,
    overrides: &HistoryOverridesFile,
    common: &CommonArgs,
) {
    if overrides.monthly_overrides.is_empty() {
        return;
    }

    let selected = common.selected_sources();
    let has_source_filter = !selected.is_empty();

    let mut added_any = false;
    for (month_key, data) in &overrides.monthly_overrides {
        // If the user filtered by source, verify this override contains models from the selected sources
        if has_source_filter {
            let matches_source = data.models.iter().any(|m| {
                infer_model_source(m).is_some_and(|src| selected.contains(&src))
            });
            if !matches_source {
                continue;
            }
        }

        // Check if specific source exclusion flags are set
        if common.no_claude
            && data
                .models
                .iter()
                .all(|m| infer_model_source(m) == Some(SourceKind::Claude))
        {
            continue;
        }
        if common.no_codex
            && data
                .models
                .iter()
                .all(|m| infer_model_source(m) == Some(SourceKind::Codex))
        {
            continue;
        }
        if common.no_gemini
            && data
                .models
                .iter()
                .all(|m| infer_model_source(m) == Some(SourceKind::Gemini))
        {
            continue;
        }
        if common.no_grok
            && data
                .models
                .iter()
                .all(|m| infer_model_source(m) == Some(SourceKind::Grok))
        {
            continue;
        }
        if common.no_opencode
            && data
                .models
                .iter()
                .all(|m| infer_model_source(m) == Some(SourceKind::OpenCode))
        {
            continue;
        }

        let existing = rows.iter_mut().find(|r| r.date == *month_key);

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
                    row.models
                        .entry(m.clone())
                        .or_insert(TokenCounts::default());
                }
                if row.sources.is_empty() {
                    row.sources
                        .insert("override".to_string(), row.totals.clone());
                }
            }
        } else {
            let mut models = BTreeMap::new();
            for m in &data.models {
                models.insert(m.clone(), TokenCounts::default());
            }
            let mut sources = BTreeMap::new();
            let totals = TokenCounts {
                input_tokens: data.input_tokens,
                cache_creation_input_tokens: data.cache_creation_input_tokens,
                cache_read_input_tokens: data.cache_read_input_tokens,
                output_tokens: data.output_tokens,
                reasoning_output_tokens: 0,
                total_tokens: data.total_tokens,
                cost_usd: data.cost_usd,
            };
            sources.insert("override".to_string(), totals.clone());

            rows.push(DailyRow {
                date: month_key.clone(),
                totals,
                models,
                sources,
                models_by_source: BTreeMap::new(),
                activity: None,
            });
            added_any = true;
        }
    }

    if added_any {
        rows.sort_by(|a, b| a.date.cmp(&b.date));
    }
}

/// Load recorded history from SQLite history.db.
pub fn load_daily_history() -> BTreeMap<String, HistoryOverrideData> {
    let mut map = BTreeMap::new();
    let Ok(conn) = init_history_db() else {
        return map;
    };
    let mut stmt = match conn.prepare(
        "SELECT date, input_tokens, cache_creation_tokens, cache_read_tokens, output_tokens, total_tokens, cost_usd FROM daily_history"
    ) {
        Ok(s) => s,
        Err(_) => return map,
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            HistoryOverrideData {
                input_tokens: row.get::<_, i64>(1)? as u64,
                cache_creation_input_tokens: row.get::<_, i64>(2)? as u64,
                cache_read_input_tokens: row.get::<_, i64>(3)? as u64,
                output_tokens: row.get::<_, i64>(4)? as u64,
                total_tokens: row.get::<_, i64>(5)? as u64,
                cost_usd: row.get::<_, f64>(6)?,
                models: vec![],
                note: None,
            },
        ))
    }) {
        Ok(r) => r,
        Err(_) => return map,
    };
    for r in rows.flatten() {
        map.insert(r.0, r.1);
    }
    map
}

use crate::ReportPeriod;
use crate::pipeline::week_start;
use crate::cli::WeekStart;

/// Merge persisted daily history from SQLite history.db into current report rows.
pub fn merge_history_db(
    rows: &mut Vec<DailyRow>,
    period: ReportPeriod,
    common: &CommonArgs,
) {
    if common.no_history_db {
        return;
    }
    let map = load_daily_history();
    if map.is_empty() {
        return;
    }

    let selected = common.selected_sources();
    if !selected.is_empty() && selected.len() < SourceKind::all().len() {
        return;
    }
    if common.no_claude || common.no_codex || common.no_gemini || common.no_grok || common.no_opencode {
        return;
    }

    let mut added_any = false;
    match period {
        ReportPeriod::Daily => {
            for (date_key, data) in map {
                if date_key.len() != 10 || date_key.contains('|') {
                    continue;
                }
                if let Some(row) = rows.iter_mut().find(|r| r.date == date_key) {
                    if row.totals.total_tokens < data.total_tokens {
                        row.totals.input_tokens = row.totals.input_tokens.max(data.input_tokens);
                        row.totals.cache_creation_input_tokens =
                            row.totals.cache_creation_input_tokens.max(data.cache_creation_input_tokens);
                        row.totals.cache_read_input_tokens =
                            row.totals.cache_read_input_tokens.max(data.cache_read_input_tokens);
                        row.totals.output_tokens = row.totals.output_tokens.max(data.output_tokens);
                        row.totals.total_tokens = row.totals.total_tokens.max(data.total_tokens);
                        row.totals.cost_usd = row.totals.cost_usd.max(data.cost_usd);
                    }
                } else {
                    let totals = TokenCounts {
                        input_tokens: data.input_tokens,
                        cache_creation_input_tokens: data.cache_creation_input_tokens,
                        cache_read_input_tokens: data.cache_read_input_tokens,
                        output_tokens: data.output_tokens,
                        reasoning_output_tokens: 0,
                        total_tokens: data.total_tokens,
                        cost_usd: data.cost_usd,
                    };
                    let mut sources = BTreeMap::new();
                    sources.insert("history.db".to_string(), totals.clone());
                    rows.push(DailyRow {
                        date: date_key,
                        totals,
                        models: BTreeMap::new(),
                        sources,
                        models_by_source: BTreeMap::new(),
                        activity: None,
                    });
                    added_any = true;
                }
            }
        }
        ReportPeriod::Monthly => {
            let mut monthly_db_direct: BTreeMap<String, TokenCounts> = BTreeMap::new();
            let mut monthly_db_daily_sums: BTreeMap<String, TokenCounts> = BTreeMap::new();

            for (date_key, data) in map {
                if date_key.contains('|') {
                    continue;
                }
                if date_key.len() == 7 && date_key.contains('-') {
                    // Direct monthly row in history.db
                    let entry = monthly_db_direct.entry(date_key.clone()).or_default();
                    entry.input_tokens = entry.input_tokens.max(data.input_tokens);
                    entry.cache_creation_input_tokens =
                        entry.cache_creation_input_tokens.max(data.cache_creation_input_tokens);
                    entry.cache_read_input_tokens =
                        entry.cache_read_input_tokens.max(data.cache_read_input_tokens);
                    entry.output_tokens = entry.output_tokens.max(data.output_tokens);
                    entry.total_tokens = entry.total_tokens.max(data.total_tokens);
                    entry.cost_usd = entry.cost_usd.max(data.cost_usd);
                } else if date_key.len() == 10 && date_key.contains('-') {
                    // Daily row to be summed by month
                    let month = date_key[..7].to_string();
                    let entry = monthly_db_daily_sums.entry(month).or_default();
                    entry.input_tokens += data.input_tokens;
                    entry.cache_creation_input_tokens += data.cache_creation_input_tokens;
                    entry.cache_read_input_tokens += data.cache_read_input_tokens;
                    entry.output_tokens += data.output_tokens;
                    entry.total_tokens += data.total_tokens;
                    entry.cost_usd += data.cost_usd;
                }
            }

            let mut all_months = monthly_db_direct.keys().cloned().collect::<std::collections::BTreeSet<_>>();
            for k in monthly_db_daily_sums.keys() {
                all_months.insert(k.clone());
            }

            for month_key in all_months {
                let direct = monthly_db_direct.get(&month_key);
                let daily_sum = monthly_db_daily_sums.get(&month_key);

                let mut db_totals = TokenCounts::default();
                if let Some(d) = direct {
                    db_totals.input_tokens = db_totals.input_tokens.max(d.input_tokens);
                    db_totals.cache_creation_input_tokens =
                        db_totals.cache_creation_input_tokens.max(d.cache_creation_input_tokens);
                    db_totals.cache_read_input_tokens =
                        db_totals.cache_read_input_tokens.max(d.cache_read_input_tokens);
                    db_totals.output_tokens = db_totals.output_tokens.max(d.output_tokens);
                    db_totals.total_tokens = db_totals.total_tokens.max(d.total_tokens);
                    db_totals.cost_usd = db_totals.cost_usd.max(d.cost_usd);
                }
                if let Some(ds) = daily_sum {
                    db_totals.input_tokens = db_totals.input_tokens.max(ds.input_tokens);
                    db_totals.cache_creation_input_tokens =
                        db_totals.cache_creation_input_tokens.max(ds.cache_creation_input_tokens);
                    db_totals.cache_read_input_tokens =
                        db_totals.cache_read_input_tokens.max(ds.cache_read_input_tokens);
                    db_totals.output_tokens = db_totals.output_tokens.max(ds.output_tokens);
                    db_totals.total_tokens = db_totals.total_tokens.max(ds.total_tokens);
                    db_totals.cost_usd = db_totals.cost_usd.max(ds.cost_usd);
                }

                if let Some(row) = rows.iter_mut().find(|r| r.date == month_key) {
                    if row.totals.total_tokens < db_totals.total_tokens {
                        row.totals.input_tokens = row.totals.input_tokens.max(db_totals.input_tokens);
                        row.totals.cache_creation_input_tokens =
                            row.totals.cache_creation_input_tokens.max(db_totals.cache_creation_input_tokens);
                        row.totals.cache_read_input_tokens =
                            row.totals.cache_read_input_tokens.max(db_totals.cache_read_input_tokens);
                        row.totals.output_tokens = row.totals.output_tokens.max(db_totals.output_tokens);
                        row.totals.total_tokens = row.totals.total_tokens.max(db_totals.total_tokens);
                        row.totals.cost_usd = row.totals.cost_usd.max(db_totals.cost_usd);
                    }
                } else {
                    let mut sources = BTreeMap::new();
                    sources.insert("history.db".to_string(), db_totals.clone());
                    rows.push(DailyRow {
                        date: month_key,
                        totals: db_totals,
                        models: BTreeMap::new(),
                        sources,
                        models_by_source: BTreeMap::new(),
                        activity: None,
                    });
                    added_any = true;
                }
            }
        }
        ReportPeriod::Weekly => {
            let mut weekly_db: BTreeMap<String, TokenCounts> = BTreeMap::new();
            for (date_key, data) in map {
                if let Ok(d) = chrono::NaiveDate::parse_from_str(&date_key, "%Y-%m-%d") {
                    let w_start = week_start(d, WeekStart::default());
                    let week_key = format!("{}", w_start.format("%Y-%m-%d"));
                    let entry = weekly_db.entry(week_key).or_default();
                    entry.input_tokens += data.input_tokens;
                    entry.cache_creation_input_tokens += data.cache_creation_input_tokens;
                    entry.cache_read_input_tokens += data.cache_read_input_tokens;
                    entry.output_tokens += data.output_tokens;
                    entry.total_tokens += data.total_tokens;
                    entry.cost_usd += data.cost_usd;
                }
            }
            for (week_key, db_totals) in weekly_db {
                if let Some(row) = rows.iter_mut().find(|r| r.date == week_key) {
                    if row.totals.total_tokens < db_totals.total_tokens {
                        row.totals.input_tokens = row.totals.input_tokens.max(db_totals.input_tokens);
                        row.totals.cache_creation_input_tokens =
                            row.totals.cache_creation_input_tokens.max(db_totals.cache_creation_input_tokens);
                        row.totals.cache_read_input_tokens =
                            row.totals.cache_read_input_tokens.max(db_totals.cache_read_input_tokens);
                        row.totals.output_tokens = row.totals.output_tokens.max(db_totals.output_tokens);
                        row.totals.total_tokens = row.totals.total_tokens.max(db_totals.total_tokens);
                        row.totals.cost_usd = row.totals.cost_usd.max(db_totals.cost_usd);
                    }
                } else {
                    let mut sources = BTreeMap::new();
                    sources.insert("history.db".to_string(), db_totals.clone());
                    rows.push(DailyRow {
                        date: week_key,
                        totals: db_totals,
                        models: BTreeMap::new(),
                        sources,
                        models_by_source: BTreeMap::new(),
                        activity: None,
                    });
                    added_any = true;
                }
            }
        }
    }

    if added_any {
        rows.sort_by(|a, b| a.date.cmp(&b.date));
    }
}

/// Initialize SQLite history.db table schema if needed.
pub fn init_history_db() -> Result<Connection> {
    let path = get_history_db_path().context("Could not locate home directory for history.db")?;
    let conn = Connection::open(path)?;
    let _ = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA mmap_size=67108864;
         PRAGMA busy_timeout=3000;",
    );
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
    if rows.is_empty() {
        return Ok(());
    }
    let Ok(mut conn) = init_history_db() else {
        return Ok(());
    };
    let now = chrono::Utc::now().to_rfc3339();

    let Ok(tx) = conn.transaction() else {
        return Ok(());
    };

    let mut stmt = tx.prepare(
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
    )?;

    for row in rows {
        // Skip instance-prefixed keys like "project | 2026-08-10" to keep daily_history clean
        if row.date.contains('|') {
            continue;
        }
        let _ = stmt.execute(params![
            row.date,
            row.totals.input_tokens as i64,
            row.totals.cache_creation_input_tokens as i64,
            row.totals.cache_read_input_tokens as i64,
            row.totals.output_tokens as i64,
            row.totals.total_tokens as i64,
            row.totals.cost_usd,
            now
        ]);
    }
    drop(stmt);
    let _ = tx.commit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_monthly_overrides_updates_lower_row() {
        let mut overrides = HistoryOverridesFile::default();
        overrides.monthly_overrides.insert(
            "2026-03".to_string(),
            HistoryOverrideData {
                total_tokens: 1548396561,
                cost_usd: 1255.63,
                input_tokens: 3001404,
                output_tokens: 2566587,
                cache_creation_input_tokens: 111025080,
                cache_read_input_tokens: 1431803490,
                models: vec!["claude-opus-4-6".to_string()],
                note: None,
            },
        );

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

        apply_monthly_overrides_with_data(&mut rows, &overrides);

        let row_2026_03 = rows.iter().find(|r| r.date == "2026-03").unwrap();
        assert_eq!(row_2026_03.totals.total_tokens, 1548396561);
        assert_eq!(row_2026_03.totals.cost_usd, 1255.63);
        assert!(row_2026_03.sources.contains_key("override"));
    }
}

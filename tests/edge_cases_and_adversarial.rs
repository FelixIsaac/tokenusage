#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn make_adversarial_env(test_name: &str) -> (PathBuf, PathBuf) {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("test-adv-{}-{}", test_name, std::process::id()));
    let _ = fs::remove_dir_all(&root);

    let claude_dir = root.join("claude/projects/-Users-test-Projects-adversarial");
    fs::create_dir_all(&claude_dir).unwrap();

    (root, claude_dir)
}

#[test]
fn test_adversarial_corrupted_json_lines_skipped_gracefully() {
    let (root, claude_dir) = make_adversarial_env("corrupted-lines");

    let corrupt_content = r#"not json at all
{"type":"assistant","truncated_json...
{"type":"assistant","timestamp":"2026-08-28T06:00:00.000Z","messageId":"m1","requestId":"r1","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":100}}}
{"type":"assistant","usage":{"input_tokens": -9999}}
{"type":"assistant","timestamp":"2026-08-28T07:00:00.000Z","messageId":"m2","requestId":"r2","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":500,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":50}}}
"#;

    fs::write(claude_dir.join("corrupt.jsonl"), corrupt_content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args([
            "daily",
            "--json",
            "--no-history-db",
            "--no-incremental-cache",
            "--claude-projects-dir",
            claude_dir.parent().unwrap().to_str().unwrap(),
            "--no-codex",
            "--no-gemini",
            "--no-grok",
            "--no-opencode",
        ])
        .env("HOME", root.join("home"))
        .output()
        .unwrap();

    assert!(output.status.success(), "Parser must not crash on corrupt lines");
    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Valid lines should still be accumulated (1000 + 500 = 1500 input tokens)
    let total_input = parsed["totals"]["input_tokens"].as_u64().unwrap();
    assert_eq!(total_input, 1500, "Valid lines should be preserved despite corrupt lines");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_empty_directory_returns_zero_or_handles_gracefully() {
    let (root, claude_dir) = make_adversarial_env("empty-dir");

    // Empty project dir with 0 files
    let output = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args([
            "daily",
            "--json",
            "--no-history-db",
            "--no-incremental-cache",
            "--claude-projects-dir",
            claude_dir.parent().unwrap().to_str().unwrap(),
            "--no-codex",
            "--no-gemini",
            "--no-grok",
            "--no-opencode",
        ])
        .env("HOME", root.join("home"))
        .output()
        .unwrap();

    assert!(output.status.success(), "Empty directory should succeed with empty/zero report");
    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["totals"]["total_tokens"].as_u64().unwrap(), 0);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_sqlite_history_db_date_filtering_end_to_end() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("test-hist-db-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);

    let home_dir = root.join("home");
    let db_dir = home_dir.join(".config/tokenusage");
    fs::create_dir_all(&db_dir).unwrap();

    let claude_dir = root.join("claude/projects/-Users-test-Projects-app");
    fs::create_dir_all(&claude_dir).unwrap();

    // Create a populated history.db with past dates from Jan 2026 and August 2026
    let conn = rusqlite::Connection::open(db_dir.join("history.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE daily_history (
            date TEXT PRIMARY KEY,
            input_tokens INTEGER NOT NULL,
            cache_creation_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            cost_usd REAL NOT NULL,
            updated_at TEXT NOT NULL
        );
        INSERT INTO daily_history VALUES ('2026-01-09', 1000, 0, 0, 100, 1100, 0.05, '2026-08-28T00:00:00Z');
        INSERT INTO daily_history VALUES ('2026-07-28', 2000, 0, 0, 200, 2200, 0.10, '2026-08-28T00:00:00Z');
        INSERT INTO daily_history VALUES ('2026-08-15', 3000, 0, 0, 300, 3300, 0.15, '2026-08-28T00:00:00Z');",
    ).unwrap();

    let empty_dir = root.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();

    // Run tu daily --since 2026-07-28 --until 2026-08-01 with history.db enabled
    let output = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args([
            "daily",
            "--since",
            "2026-07-28",
            "--until",
            "2026-08-01",
            "--json",
            "--claude-projects-dir",
            empty_dir.to_str().unwrap(),
            "--codex-sessions-dir",
            empty_dir.to_str().unwrap(),
            "--gemini-data-dir",
            empty_dir.to_str().unwrap(),
            "--grok-log-dir",
            empty_dir.to_str().unwrap(),
            "--opencode-data-dir",
            empty_dir.to_str().unwrap(),
        ])
        .env("HOME", &home_dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let daily_rows = parsed["daily"].as_array().unwrap();
    // Must contain ONLY 2026-07-28, not 2026-01-09 and not 2026-08-15!
    assert_eq!(daily_rows.len(), 1, "Must contain exactly 1 row matching the since/until range");
    assert_eq!(daily_rows[0]["date"].as_str().unwrap(), "2026-07-28");

    let _ = fs::remove_dir_all(&root);
}

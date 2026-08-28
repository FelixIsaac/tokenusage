#![cfg(unix)]

mod common;

use common::MultiYearFixture;

#[test]
fn test_multi_year_monthly_aggregation_and_ordering() {
    let fixture = MultiYearFixture::new("monthly-agg");
    fixture.populate_multi_year_data();

    // Query 3 full years: 2024 to 2026
    let output = fixture.cli_cmd(&[
        "monthly",
        "--since",
        "2024-01-01",
        "--until",
        "2026-12-31",
        "--json",
    ]);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let rows = json["monthly"].as_array().expect("monthly rows array");
    assert!(!rows.is_empty(), "must have monthly rows");

    let months: Vec<&str> = rows.iter().map(|r| r["date"].as_str().unwrap()).collect();

    // Must have records from 2024, 2025, and 2026
    assert!(months.iter().any(|m| m.starts_with("2024-")), "missing 2024 data");
    assert!(months.iter().any(|m| m.starts_with("2025-")), "missing 2025 data");
    assert!(months.iter().any(|m| m.starts_with("2026-")), "missing 2026 data");

    // Chronological order verification
    let mut sorted = months.clone();
    sorted.sort();
    assert_eq!(months, sorted, "months must be sorted chronologically");

    // Totals check
    let totals = &json["totals"];
    assert!(totals["total_tokens"].as_u64().unwrap() > 100_000);
    assert!(totals["cost_usd"].as_f64().unwrap() > 5.0);
}

#[test]
fn test_leap_year_handling_feb_29() {
    let fixture = MultiYearFixture::new("leap-year");
    fixture.populate_multi_year_data();

    // Range strictly covering leap day 2024-02-29
    let output = fixture.cli_cmd(&[
        "daily",
        "--since",
        "2024-02-28",
        "--until",
        "2024-03-01",
        "--json",
        "--no-history-db",
    ]);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let rows = json["daily"].as_array().unwrap();
    let dates: Vec<&str> = rows.iter().map(|r| r["date"].as_str().unwrap()).collect();

    assert!(dates.contains(&"2024-02-29"), "Leap day 2024-02-29 must be present in parsed results");
    assert!(dates.contains(&"2024-03-01"), "2024-03-01 must be present");
    assert_eq!(dates.len(), 2, "Only dates in the 2024-02-28..2024-03-01 range should be present");
}

#[test]
fn test_year_boundary_weekly_rollover() {
    let fixture = MultiYearFixture::new("year-boundary-weekly");
    fixture.populate_multi_year_data();

    // Range crossing 2025 -> 2026 year boundary
    let output = fixture.cli_cmd(&[
        "weekly",
        "--since",
        "2025-12-25",
        "--until",
        "2026-01-08",
        "--json",
        "--no-history-db",
    ]);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let rows = json["weekly"].as_array().unwrap();
    assert!(!rows.is_empty(), "Week crossing year boundary must be present");

    // Both Dec 31 2025 and Jan 1 2026 events should be captured
    let total_tokens = json["totals"]["total_tokens"].as_u64().unwrap();
    assert!(total_tokens > 0);
}

#[test]
fn test_multi_year_history_db_range_isolation() {
    let fixture = MultiYearFixture::new("history-isolation");
    fixture.populate_multi_year_data();

    // Filter to 2025 only: should include 2025 history.db rows but exclude 2024 and 2026
    let output = fixture.cli_cmd(&[
        "monthly",
        "--since",
        "2025-01-01",
        "--until",
        "2025-12-31",
        "--json",
    ]);

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let rows = json["monthly"].as_array().unwrap();
    let months: Vec<&str> = rows.iter().map(|r| r["date"].as_str().unwrap()).collect();

    for m in &months {
        assert!(m.starts_with("2025-"), "Found out-of-range month in 2025 query: {m}");
    }
}

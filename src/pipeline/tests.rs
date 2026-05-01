use super::live::resolve_live_block_bounds;
use super::official::{
    extract_flag_value, normalize_official_used_percent, parse_antigravity_command_model_configs,
    parse_antigravity_reset_time, parse_antigravity_user_status, select_antigravity_models,
};
use super::parsing::dedupe_opencode_events;
use super::statusline::active_block_summary_for_bounds;
use super::*;
use chrono::TimeZone;

fn utc_dt(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .unwrap()
}

fn test_event(ts: DateTime<Utc>, source: SourceKind, total_tokens: u64) -> UsageEvent {
    UsageEvent {
        timestamp: ts,
        source,
        model: "gpt-5.3-codex".to_string(),
        session: "s".to_string(),
        project: None,
        file_path: "/tmp/log.jsonl".to_string(),
        usage: UsageAccumulator {
            input_tokens: total_tokens,
            ..UsageAccumulator::default()
        },
    }
}

#[test]
fn normalize_official_percent_treats_one_as_percent_not_ratio() {
    assert!((normalize_official_used_percent(0.82) - 82.0).abs() < f64::EPSILON);
    assert!((normalize_official_used_percent(82.0) - 82.0).abs() < f64::EPSILON);
    assert!((normalize_official_used_percent(1.0) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn resolve_live_bounds_aligns_to_official_reset_window() {
    let now = utc_dt(2026, 3, 3, 16, 7, 36);
    let official = OfficialCodexSnapshot {
        plan_type: Some("pro".to_string()),
        primary_used_percent: Some(99.0),
        secondary_used_percent: Some(82.0),
        primary_window_mins: Some(300),
        secondary_window_mins: Some(10080),
        primary_resets_at: Some(utc_dt(2026, 3, 3, 20, 59, 47).timestamp()),
        secondary_resets_at: Some(utc_dt(2026, 3, 10, 10, 59, 47).timestamp()),
    };

    let (start, end, window_secs) = resolve_live_block_bounds(
        now,
        5 * 3600,
        Some(SourceKind::Codex),
        Some(&official),
        None,
    );

    assert_eq!(window_secs, 5 * 3600);
    assert_eq!(start, utc_dt(2026, 3, 3, 15, 59, 47).timestamp());
    assert_eq!(end, utc_dt(2026, 3, 3, 20, 59, 47).timestamp());
    assert!(now.timestamp() >= start && now.timestamp() < end);
}

#[test]
fn resolve_live_bounds_rolls_old_reset_forward_to_current_session() {
    let now = utc_dt(2026, 3, 3, 16, 7, 36);
    let stale = OfficialCodexSnapshot {
        plan_type: Some("pro".to_string()),
        primary_used_percent: Some(99.0),
        secondary_used_percent: Some(82.0),
        primary_window_mins: Some(300),
        secondary_window_mins: Some(10080),
        // previous session boundary; function should advance it.
        primary_resets_at: Some(utc_dt(2026, 3, 3, 10, 59, 47).timestamp()),
        secondary_resets_at: None,
    };

    let (start, end, window_secs) =
        resolve_live_block_bounds(now, 5 * 3600, Some(SourceKind::Codex), Some(&stale), None);

    assert_eq!(window_secs, 5 * 3600);
    assert_eq!(start, utc_dt(2026, 3, 3, 15, 59, 47).timestamp());
    assert_eq!(end, utc_dt(2026, 3, 3, 20, 59, 47).timestamp());
}

#[test]
fn active_block_summary_for_bounds_only_counts_events_in_current_window() {
    let now = utc_dt(2026, 3, 3, 16, 30, 0);
    let block_start = utc_dt(2026, 3, 3, 15, 59, 47).timestamp();
    let block_end = utc_dt(2026, 3, 3, 20, 59, 47).timestamp();

    let events = vec![
        // previous window event, must be excluded
        test_event(utc_dt(2026, 3, 3, 15, 30, 0), SourceKind::Codex, 900),
        // current window events
        test_event(utc_dt(2026, 3, 3, 16, 0, 0), SourceKind::Codex, 100),
        test_event(utc_dt(2026, 3, 3, 16, 10, 0), SourceKind::Codex, 300),
    ];

    let summary = active_block_summary_for_bounds(&events, now, block_start, block_end)
        .expect("expected active summary for current window");

    assert_eq!(summary.totals.total_tokens, 400);
    assert_eq!(summary.dominant_source, Some(SourceKind::Codex));
    assert!(summary.remaining_minutes >= 0);
}

// ---- Antigravity tests ----

#[test]
fn parse_antigravity_user_status_response() {
    let json = serde_json::json!({
        "code": 0,
        "userStatus": {
            "email": "test@example.com",
            "planStatus": {
                "planInfo": {
                    "planName": "pro",
                    "planDisplayName": "Pro Plan",
                    "displayName": "Pro",
                    "productName": "Antigravity Pro",
                    "planShortName": "pro"
                }
            },
            "cascadeModelConfigData": {
                "clientModelConfigs": [
                    {
                        "label": "Claude 4 Sonnet",
                        "modelOrAlias": { "model": "claude-4-sonnet" },
                        "quotaInfo": {
                            "remainingFraction": 0.75,
                            "resetTime": "1709500800"
                        }
                    },
                    {
                        "label": "Gemini Pro Low",
                        "modelOrAlias": { "model": "gemini-pro" },
                        "quotaInfo": {
                            "remainingFraction": 0.50,
                            "resetTime": "1709500800"
                        }
                    },
                    {
                        "label": "Gemini 2.5 Flash",
                        "modelOrAlias": { "model": "gemini-flash" },
                        "quotaInfo": {
                            "remainingFraction": 0.90
                        }
                    }
                ]
            }
        }
    });

    let snapshot = parse_antigravity_user_status(&json).unwrap();
    assert_eq!(snapshot.plan_type.as_deref(), Some("Pro Plan"));
    assert_eq!(snapshot.account_email.as_deref(), Some("test@example.com"));
    assert_eq!(snapshot.models.len(), 3);

    // Claude is primary (25% used)
    assert!((snapshot.primary_used_percent.unwrap() - 25.0).abs() < 0.01);
    assert_eq!(snapshot.primary_label.as_deref(), Some("Claude 4 Sonnet"));

    // Gemini Pro Low is secondary (50% used)
    assert!((snapshot.secondary_used_percent.unwrap() - 50.0).abs() < 0.01);
    assert_eq!(snapshot.secondary_label.as_deref(), Some("Gemini Pro Low"));

    // Gemini Flash is tertiary (10% used)
    assert!((snapshot.tertiary_used_percent.unwrap() - 10.0).abs() < 0.01);
    assert_eq!(snapshot.tertiary_label.as_deref(), Some("Gemini 2.5 Flash"));
}

#[test]
fn parse_antigravity_command_model_response() {
    let json = serde_json::json!({
        "code": "ok",
        "clientModelConfigs": [
            {
                "label": "GPT-4o",
                "modelOrAlias": { "model": "gpt-4o" },
                "quotaInfo": {
                    "remainingFraction": 0.30
                }
            }
        ]
    });

    let snapshot = parse_antigravity_command_model_configs(&json).unwrap();
    assert!(snapshot.plan_type.is_none());
    assert!(snapshot.account_email.is_none());
    assert_eq!(snapshot.models.len(), 1);
    // Fallback: single model becomes primary
    assert!((snapshot.primary_used_percent.unwrap() - 70.0).abs() < 0.01);
    assert_eq!(snapshot.primary_label.as_deref(), Some("GPT-4o"));
}

#[test]
fn select_antigravity_models_prioritises_correctly() {
    let models = vec![
        AntigravityModelQuotaSnapshot {
            label: "Gemini 2.5 Flash".to_string(),
            model_id: "gemini-flash".to_string(),
            remaining_fraction: Some(0.9),
            reset_time: None,
        },
        AntigravityModelQuotaSnapshot {
            label: "Claude 4 Sonnet".to_string(),
            model_id: "claude-4-sonnet".to_string(),
            remaining_fraction: Some(0.5),
            reset_time: None,
        },
        AntigravityModelQuotaSnapshot {
            label: "Gemini Pro Low".to_string(),
            model_id: "gemini-pro-low".to_string(),
            remaining_fraction: Some(0.3),
            reset_time: None,
        },
    ];

    let ordered = select_antigravity_models(&models);
    assert_eq!(ordered.len(), 3);
    assert_eq!(ordered[0].label, "Claude 4 Sonnet");
    assert_eq!(ordered[1].label, "Gemini Pro Low");
    assert_eq!(ordered[2].label, "Gemini 2.5 Flash");
}

#[test]
fn select_antigravity_models_fallback_sorts_by_remaining() {
    let models = vec![
        AntigravityModelQuotaSnapshot {
            label: "Model A".to_string(),
            model_id: "a".to_string(),
            remaining_fraction: Some(0.8),
            reset_time: None,
        },
        AntigravityModelQuotaSnapshot {
            label: "Model B".to_string(),
            model_id: "b".to_string(),
            remaining_fraction: Some(0.2),
            reset_time: None,
        },
    ];

    let ordered = select_antigravity_models(&models);
    assert_eq!(ordered[0].label, "Model B"); // 0.2 remaining = lowest first
    assert_eq!(ordered[1].label, "Model A");
}

#[test]
fn extract_flag_value_works() {
    let cmd = "/path/to/language_server_macos --csrf_token=abc123 --extension_server_port 42150 --app_data_dir antigravity";
    assert_eq!(
        extract_flag_value("--csrf_token", cmd),
        Some("abc123".to_string())
    );
    assert_eq!(
        extract_flag_value("--extension_server_port", cmd),
        Some("42150".to_string())
    );
    assert_eq!(
        extract_flag_value("--app_data_dir", cmd),
        Some("antigravity".to_string())
    );
    assert_eq!(extract_flag_value("--nonexistent", cmd), None);
}

#[test]
fn parse_antigravity_reset_time_iso8601() {
    let ts = parse_antigravity_reset_time("2024-03-04T12:00:00Z");
    assert!(ts.is_some());
    assert_eq!(ts.unwrap(), 1709553600);
}

#[test]
fn parse_antigravity_reset_time_epoch() {
    let ts = parse_antigravity_reset_time("1709500800");
    assert_eq!(ts, Some(1709500800));
}

#[test]
fn parse_antigravity_error_code_rejects_nonzero() {
    let json = serde_json::json!({
        "code": 7,
        "userStatus": null,
    });
    assert!(parse_antigravity_user_status(&json).is_err());
}

#[test]
fn opencode_dedupe_removes_duplicate_signatures_only() {
    let ts = utc_dt(2026, 4, 1, 10, 0, 0);
    let base = UsageEvent {
        timestamp: ts,
        source: SourceKind::OpenCode,
        model: "gpt-5".to_string(),
        session: "s1".to_string(),
        project: Some("p1".to_string()),
        file_path: "a.json".to_string(),
        usage: UsageAccumulator {
            input_tokens: 10,
            output_tokens: 5,
            ..UsageAccumulator::default()
        },
    };
    let mut events = vec![
        base.clone(),
        UsageEvent {
            file_path: "b.json".to_string(),
            ..base.clone()
        },
        UsageEvent {
            source: SourceKind::Codex,
            ..base.clone()
        },
        UsageEvent {
            usage: UsageAccumulator {
                input_tokens: 11,
                output_tokens: 5,
                ..UsageAccumulator::default()
            },
            ..base
        },
    ];

    dedupe_opencode_events(&mut events);
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.source == SourceKind::OpenCode)
            .count(),
        2
    );
}

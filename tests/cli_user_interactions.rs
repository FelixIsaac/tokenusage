#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn setup_fixture(test_name: &str) -> (PathBuf, PathBuf) {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "test-cli-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);

    let claude_dir = root.join("claude/projects/-Users-test-Projects-core");
    fs::create_dir_all(&claude_dir).unwrap();

    let sample_line = r#"{"type":"assistant","timestamp":"2026-08-28T10:00:00.000Z","messageId":"m1","requestId":"r1","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":1000,"cache_creation_input_tokens":200,"cache_read_input_tokens":500,"output_tokens":150}}}"#;
    fs::write(
        claude_dir.join("session.jsonl"),
        format!("{}\n", sample_line),
    )
    .unwrap();

    (root.clone(), root.join("claude/projects"))
}

#[test]
fn test_cli_usage_trend_compares_adjacent_completed_periods() {
    let (root, claude_dir) = setup_fixture("usage-trend");
    let sonnet = "claude-3-7-sonnet-20250219";
    let opus = "claude-opus-4-8";
    let days = [
        ("2026-07-31", sonnet, 100_000, 200_000, 50_000, 50_000),
        ("2026-08-30", sonnet, 50_000, 20_000, 10_000, 20_000),
        ("2026-08-31", sonnet, 50_000, 20_000, 10_000, 20_000),
        ("2026-09-06", opus, 400_000, 0, 0, 0),
        ("2026-09-13", opus, 50_000, 0, 0, 0),
    ];
    let events = days.map(|(day, model, input, reads, writes, output)| {
        serde_json::json!({
            "type": "assistant",
            "timestamp": format!("{day}T10:00:00Z"),
            "messageId": day,
            "requestId": day,
            "message": {
                "model": model,
                "usage": {
                    "input_tokens": input,
                    "cache_read_input_tokens": reads,
                    "cache_creation_input_tokens": writes,
                    "output_tokens": output
                }
            }
        })
        .to_string()
    });
    fs::write(
        claude_dir.join("-Users-test-Projects-core/session.jsonl"),
        events.join("\n"),
    )
    .unwrap();

    let run = |args: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_tu"))
            .args(args)
            .args([
                "--offline",
                "--no-history-db",
                "--no-history-overrides",
                "--no-incremental-cache",
                "--sources",
                "claude",
                "--timezone",
                "UTC",
                "--claude-projects-dir",
                claude_dir.to_str().unwrap(),
            ])
            .env("HOME", root.join("home"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    let trend = |args: &[&str]| {
        serde_json::from_str::<serde_json::Value>(&run(args)).unwrap()["insights"]["trend"].clone()
    };

    let monthly = trend(&[
        "monthly",
        "--until",
        "2026-09-14",
        "--json",
        "--order",
        "desc",
    ]);
    assert_eq!(monthly["previous"]["date"], "2026-07");
    assert_eq!(monthly["latest"]["date"], "2026-08");
    assert_eq!(monthly["token_change_pct"], -50.0);
    assert_eq!(monthly["latest"]["cache_hit_pct"], 25.0);
    assert_eq!(monthly["latest"]["cache_reuse_ratio"], 2.0);
    assert!((monthly["cache_hit_change_pp"].as_f64().unwrap() + 32.142857).abs() < 1e-6);
    assert_eq!(monthly["previous"]["cache_read_input_tokens"], 200_000);
    assert_eq!(monthly["latest"]["output_tokens"], 40_000);
    assert_eq!(
        monthly["latest"]["sources"]["claude"]["total_tokens"],
        200_000
    );
    assert!(monthly["latest"]["cost_per_mtoken"].as_f64().unwrap() > 0.0);
    assert!(monthly["latest"]["cache_savings_usd"].as_f64().unwrap() > 0.0);

    let weekly = trend(&[
        "weekly",
        "--start-of-week",
        "sunday",
        "--until",
        "2026-09-14",
        "--json",
    ]);
    assert_eq!(weekly["previous"]["date"], "2026-08-30");
    assert_eq!(weekly["latest"]["date"], "2026-09-06");
    assert_eq!(weekly["token_change_pct"], 100.0);
    assert_eq!(weekly["latest"]["cache_hit_pct"], 0.0);
    assert_eq!(weekly["cache_hit_change_pp"], -25.0);
    assert_eq!(weekly["latest"]["cache_savings_usd"], 0.0);
    assert_eq!(weekly["latest"]["models"][opus]["total_tokens"], 400_000);

    let daily = trend(&["daily", "--instances", "--until", "2026-08-31", "--json"]);
    assert_eq!(daily["previous"]["date"], "2026-08-30");
    assert_eq!(daily["latest"]["date"], "2026-08-31");
    assert_eq!(daily["token_change_pct"], 0.0);
    assert!(trend(&["daily", "--until", "2026-09-06", "--json"]).is_null());
    assert!(
        trend(&[
            "monthly",
            "--since",
            "2026-07-02",
            "--until",
            "2026-09-14",
            "--json"
        ])
        .is_null()
    );

    let table = run(&["monthly", "--until", "2026-09-14"]);
    assert!(table.contains("Usage Trend"));
    assert!(table.contains("2026-07 → 2026-08"));
    assert!(table.contains("400,000 → 200,000 (-50.0%)"));
    assert!(table.contains("57.14% → 25.00% (-32.14 pp)"));
    assert!(table.contains("Est. Cache Savings"));
    assert!(table.contains("Cost / 1M tok"));
    assert!(table.contains("Largest Model Cost Change"));

    let weekly_table = run(&[
        "weekly",
        "--start-of-week",
        "sunday",
        "--until",
        "2026-09-14",
    ]);
    assert!(weekly_table.contains("Top Provider"));
    assert!(weekly_table.contains("Top Model"));
    assert!(
        weekly_table.contains("claude-3-7-sonnet-20250219 (100.0%) → claude-opus-4-8 (100.0%)")
    );
}

#[test]
fn test_cli_subcommands_happy_paths() {
    let (root, claude_dir) = setup_fixture("subcommands-happy");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let npx = bin_dir.join("npx");
    fs::write(
        &npx,
        r#"#!/bin/sh
[ "$#" -eq 4 ] && [ "$1" = "ccusage@latest" ] && [ "$2" = "daily" ] &&
    [ "$3" = "--json" ] && [ "$4" = "--offline" ] || exit 1
printf '%s\n' '{"totals":{"inputTokens":1000,"cacheCreationTokens":200,"cacheReadTokens":500,"outputTokens":150,"totalTokens":1850}}'
"#,
    )
    .unwrap();
    fs::set_permissions(npx, fs::Permissions::from_mode(0o755)).unwrap();

    let run_cmd = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tu"))
            .args(args)
            .args([
                "--offline",
                "--no-history-db",
                "--no-incremental-cache",
                "--claude-projects-dir",
                claude_dir.to_str().unwrap(),
                "--no-codex",
                "--no-gemini",
                "--no-grok",
                "--no-opencode",
            ])
            .env("HOME", root.join("home"))
            .env("PATH", &bin_dir)
            .output()
            .unwrap()
    };

    // 1. daily / days / day / d
    let res = run_cmd(&["daily", "--json"]);
    assert!(res.status.success(), "daily --json failed");
    let res = run_cmd(&["days", "--json"]);
    assert!(res.status.success(), "days alias --json failed");
    let res = run_cmd(&["day", "--json"]);
    assert!(res.status.success(), "day alias --json failed");
    let res = run_cmd(&["d", "--json"]);
    assert!(res.status.success(), "d alias --json failed");

    // 2. today / now / t
    let res = run_cmd(&["today", "--json"]);
    assert!(res.status.success(), "today --json failed");
    let res = run_cmd(&["now", "--json"]);
    assert!(res.status.success(), "now alias --json failed");
    let res = run_cmd(&["t", "--json"]);
    assert!(res.status.success(), "t alias --json failed");

    // 3. weekly / weeks / week / w
    let res = run_cmd(&["weekly", "--json"]);
    assert!(res.status.success(), "weekly --json failed");
    let res = run_cmd(&["weeks", "--json"]);
    assert!(res.status.success(), "weeks alias --json failed");
    let res = run_cmd(&["week", "--json"]);
    assert!(res.status.success(), "week alias --json failed");
    let res = run_cmd(&["w", "--json"]);
    assert!(res.status.success(), "w alias --json failed");

    // 4. monthly / months / month / m
    let res = run_cmd(&["monthly", "--json"]);
    assert!(res.status.success(), "monthly --json failed");
    let res = run_cmd(&["months", "--json"]);
    assert!(res.status.success(), "months alias --json failed");
    let res = run_cmd(&["month", "--json"]);
    assert!(res.status.success(), "month alias --json failed");
    let res = run_cmd(&["m", "--json"]);
    assert!(res.status.success(), "m alias --json failed");

    // 5. blocks / block / b
    let res = run_cmd(&["blocks", "--active", "--json"]);
    assert!(res.status.success(), "blocks --json failed");
    let res = run_cmd(&["block", "--active", "--json"]);
    assert!(res.status.success(), "block alias --json failed");
    let res = run_cmd(&["b", "--active", "--json"]);
    assert!(res.status.success(), "b alias --json failed");

    // 6. session / sessions / sess / s
    let res = run_cmd(&["session", "--json"]);
    assert!(res.status.success(), "session --json failed");
    let res = run_cmd(&["sessions", "--json"]);
    assert!(res.status.success(), "sessions alias --json failed");
    let res = run_cmd(&["sess", "--json"]);
    assert!(res.status.success(), "sess alias --json failed");
    let res = run_cmd(&["s", "--json"]);
    assert!(res.status.success(), "s alias --json failed");

    // 7. activity / act / coding
    let res = run_cmd(&["activity", "--json"]);
    assert!(res.status.success(), "activity --json failed");
    let res = run_cmd(&["act", "--json"]);
    assert!(res.status.success(), "act alias --json failed");
    let res = run_cmd(&["coding", "--json"]);
    assert!(res.status.success(), "coding alias --json failed");

    // 8. carbon / co2 / energy / footprint
    let res = run_cmd(&["carbon", "today", "--json"]);
    assert!(res.status.success(), "carbon today --json failed");
    let res = run_cmd(&["co2", "today", "--json"]);
    assert!(res.status.success(), "co2 alias --json failed");
    let res = run_cmd(&["energy", "today", "--json"]);
    assert!(res.status.success(), "energy alias --json failed");
    let res = run_cmd(&["footprint", "today", "--json"]);
    assert!(res.status.success(), "footprint alias --json failed");

    // 9. doctor / doc / check / health
    let res = run_cmd(&["doctor", "--json"]);
    assert!(res.status.success(), "doctor --json failed");
    let res = run_cmd(&["doc", "--json"]);
    assert!(res.status.success(), "doc alias --json failed");
    let res = run_cmd(&["check", "--json"]);
    assert!(res.status.success(), "check alias --json failed");
    let res = run_cmd(&["health", "--json"]);
    assert!(res.status.success(), "health alias --json failed");

    // 10. statusline / sl
    let res = run_cmd(&["statusline", "--json"]);
    assert!(res.status.success(), "statusline --json failed");
    let res = run_cmd(&["sl", "--json"]);
    assert!(res.status.success(), "sl alias --json failed");

    // 11. parity / diff / compare
    for alias in ["parity", "diff", "compare"] {
        let result = run_cmd(&[alias, "--json"]);
        assert!(
            result.status.success(),
            "{alias} --json failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let parity: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(parity["ccusage"]["total_tokens"], 1850);
        assert_eq!(parity["delta"]["total_tokens"], 0);
    }

    // 12. completions / completion
    let res = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["completions", "zsh"])
        .output()
        .unwrap();
    assert!(res.status.success());
    assert!(!res.stdout.is_empty());

    let res = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["completion", "zsh"])
        .output()
        .unwrap();
    assert!(res.status.success());
    assert!(!res.stdout.is_empty());
    assert!(!res.stdout.is_empty());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_cli_date_formats_and_filtering() {
    let (root, claude_dir) = setup_fixture("date-filtering");

    let run_filter = |since: &str, until: &str| {
        Command::new(env!("CARGO_BIN_EXE_tu"))
            .args([
                "daily",
                "--since",
                since,
                "--until",
                until,
                "--json",
                "--no-history-db",
                "--no-incremental-cache",
                "--claude-projects-dir",
                claude_dir.to_str().unwrap(),
                "--no-codex",
                "--no-gemini",
                "--no-grok",
                "--no-opencode",
            ])
            .env("HOME", root.join("home"))
            .output()
            .unwrap()
    };

    // Standard ISO format
    let res = run_filter("2026-08-01", "2026-08-31");
    assert!(res.status.success(), "ISO date format failed");

    // Slash format DD/MM/YYYY
    let res = run_filter("01/08/2026", "31/08/2026");
    assert!(res.status.success(), "DD/MM/YYYY format failed");

    // Compact YYYYMMDD
    let res = run_filter("20260801", "20260831");
    assert!(res.status.success(), "YYYYMMDD format failed");

    // Relative date formats
    let res = run_filter("30d", "today");
    assert!(res.status.success(), "30d/today format failed");

    let res = run_filter("7d", "yesterday");
    assert!(res.status.success(), "7d/yesterday format failed");

    let res = run_filter("1w", "today");
    assert!(res.status.success(), "1w/today format failed");

    let res = run_filter("1m", "today");
    assert!(res.status.success(), "1m/today format failed");

    let res = run_filter("this-month", "today");
    assert!(res.status.success(), "this-month/today format failed");

    // Inverted range should fail gracefully
    let res = run_filter("2026-08-31", "2026-08-01");
    assert!(!res.status.success(), "Inverted range must fail");
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(err.contains("--since must be earlier than or equal to --until"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_cli_unhappy_paths_and_error_handling() {
    // 1. Unrecognized subcommand
    let res = Command::new(env!("CARGO_BIN_EXE_tu"))
        .arg("invalid_command_xyz")
        .output()
        .unwrap();
    assert!(!res.status.success());
    assert!(String::from_utf8_lossy(&res.stderr).contains("unrecognized subcommand"));

    // 2. Conflicting flags: --json and --tui
    let res = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["daily", "--json", "--tui"])
        .output()
        .unwrap();
    assert!(!res.status.success());
    assert!(String::from_utf8_lossy(&res.stderr).contains("cannot be used together"));

    // 3. Invalid timezone
    let res = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["daily", "--timezone", "Invalid/Timezone_123"])
        .output()
        .unwrap();
    assert!(!res.status.success());
    assert!(String::from_utf8_lossy(&res.stderr).contains("Invalid timezone"));
}

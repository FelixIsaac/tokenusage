#![cfg(unix)]

use std::fs;
use std::path::PathBuf;

pub struct MultiYearFixture {
    pub root: PathBuf,
    pub home_dir: PathBuf,
    pub claude_dir: PathBuf,
    pub codex_dir: PathBuf,
    pub gemini_dir: PathBuf,
    pub grok_dir: PathBuf,
    pub opencode_dir: PathBuf,
    pub config_dir: PathBuf,
}

impl MultiYearFixture {
    pub fn new(test_name: &str) -> Self {
        let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("fixture-multiyear-{}-{}", test_name, std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let home_dir = root.join("home");
        let claude_dir = root.join("claude/projects");
        let codex_dir = root.join("codex/sessions");
        let gemini_dir = root.join("gemini/tmp");
        let grok_dir = root.join("grok/sessions");
        let opencode_dir = root.join("opencode/storage/message");
        let config_dir = home_dir.join(".config/tokenusage");

        fs::create_dir_all(&claude_dir).unwrap();
        fs::create_dir_all(&codex_dir).unwrap();
        fs::create_dir_all(&gemini_dir).unwrap();
        fs::create_dir_all(&grok_dir).unwrap();
        fs::create_dir_all(&opencode_dir).unwrap();
        fs::create_dir_all(&config_dir).unwrap();

        Self {
            root,
            home_dir,
            claude_dir,
            codex_dir,
            gemini_dir,
            grok_dir,
            opencode_dir,
            config_dir,
        }
    }

    pub fn populate_multi_year_data(&self) {
        self.populate_claude_multi_year();
        self.populate_codex_multi_year();
        self.populate_gemini_multi_year();
        self.populate_grok_multi_year();
        self.populate_opencode_multi_year();
        self.populate_history_db();
    }

    fn populate_claude_multi_year(&self) {
        let projects = ["backend-service", "frontend-ui", "data-pipeline"];
        let dates = [
            // 2024 leap year date
            ("2024-02-29T10:00:00Z", "claude-3-opus-20240229", 1500, 200, 800, 300),
            ("2024-07-15T14:30:00Z", "claude-3-5-sonnet-20240620", 2500, 500, 1200, 450),
            ("2024-11-20T09:15:00Z", "claude-3-5-sonnet-20241022", 3000, 600, 1500, 600),
            // 2025 dates
            ("2025-01-10T11:00:00Z", "claude-3-5-sonnet-20241022", 2000, 400, 1000, 400),
            ("2025-06-01T16:45:00Z", "claude-3-5-haiku-20241022", 1200, 100, 600, 250),
            ("2025-12-31T23:50:00Z", "claude-3-5-sonnet-20241022", 4000, 800, 2000, 800),
            // 2026 dates
            ("2026-01-01T00:10:00Z", "claude-3-7-sonnet-20250219", 3500, 700, 1800, 700),
            ("2026-04-15T12:00:00Z", "claude-3-7-sonnet-20250219", 2800, 600, 1400, 550),
            ("2026-08-28T08:00:00Z", "claude-sonnet-5", 5000, 1000, 3000, 1000),
        ];

        for (p_idx, proj) in projects.iter().enumerate() {
            let proj_dir = self.claude_dir.join(format!("-Users-felix-Projects-{}", proj));
            fs::create_dir_all(&proj_dir).unwrap();

            for (d_idx, (ts, model, in_tok, cw_tok, cr_tok, out_tok)) in dates.iter().enumerate() {
                let session_file = proj_dir.join(format!("session-{}-{}.jsonl", p_idx, d_idx));
                let line1 = r#"{"type":"user","message":{"content":"Implement new feature"}}"#;
                let line2 = format!(
                    r#"{{"type":"assistant","timestamp":"{}","messageId":"msg-{}-{}","requestId":"req-{}-{}","message":{{"model":"{}","usage":{{"input_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{},"output_tokens":{},"reasoning_output_tokens":{}}}}}}}"#,
                    ts, p_idx, d_idx, p_idx, d_idx, model, in_tok, cw_tok, cr_tok, out_tok, out_tok / 3
                );
                fs::write(session_file, format!("{}\n{}\n", line1, line2)).unwrap();
            }
        }
    }

    fn populate_codex_multi_year(&self) {
        let sessions = [
            ("2024-03-01", "2024-03-01T15:00:00Z", "gpt-4o", 2000, 1200, 350),
            ("2024-08-10", "2024-08-10T18:00:00Z", "gpt-4o-mini", 1000, 500, 200),
            ("2025-03-15", "2025-03-15T09:00:00Z", "gpt-4.5-preview", 3000, 1500, 500),
            ("2025-09-22", "2025-09-22T14:20:00Z", "gpt-5", 4000, 2500, 800),
            ("2026-02-14", "2026-02-14T11:30:00Z", "gpt-5-codex", 3500, 2000, 600),
            ("2026-08-28", "2026-08-28T07:15:00Z", "gpt-5.6-terra", 6000, 4000, 1200),
        ];

        for (idx, (folder, ts, model, in_tok, cr_tok, out_tok)) in sessions.iter().enumerate() {
            let sess_dir = self.codex_dir.join(folder);
            fs::create_dir_all(&sess_dir).unwrap();

            let file_path = sess_dir.join(format!("session-{}.jsonl", idx));
            let meta = r#"{"type":"session_meta","payload":{"cwd":"/Users/felix/Projects/codex-core"}}"#;
            let turn = format!(r#"{{"type":"turn_context","payload":{{"model":"{}"}}}}"#, model);
            let event = format!(
                r#"{{"type":"event_msg","timestamp":"{}","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":{},"cached_input_tokens":{},"output_tokens":{},"reasoning_output_tokens":{},"total_tokens":{}}}}}}}}}"#,
                ts, in_tok, cr_tok, out_tok, out_tok / 4, in_tok + out_tok
            );
            fs::write(file_path, format!("{}\n{}\n{}\n", meta, turn, event)).unwrap();
        }
    }

    fn populate_gemini_multi_year(&self) {
        let events = [
            ("2024-05-20T12:00:00Z", "gemini-1.5-pro", 5000, 3000, 600),
            ("2025-02-10T15:30:00Z", "gemini-2.0-flash", 4000, 2500, 500),
            ("2025-08-18T10:00:00Z", "gemini-2.5-pro", 6000, 4000, 800),
            ("2026-03-30T17:00:00Z", "gemini-3.6-flash", 3000, 2000, 450),
            ("2026-08-28T09:00:00Z", "gemini-3.7-flash", 4500, 3200, 700),
        ];

        for (idx, (ts, model, in_tok, cr_tok, out_tok)) in events.iter().enumerate() {
            let file_path = self.gemini_dir.join(format!("gemini-{}.jsonl", idx));
            let content = format!(
                r#"{{"type":"gemini","timestamp":"{}","model":"{}","tokens":{{"input":{},"cached":{},"output":{},"thoughts":{}}}}}"#,
                ts, model, in_tok, cr_tok, out_tok, out_tok / 3
            );
            fs::write(file_path, format!("{}\n", content)).unwrap();
        }
    }

    fn populate_grok_multi_year(&self) {
        let sessions = [
            ("2025-04-12", "019fa4ff-0001-7000-0000-000000000001", 1744459200i64, "grok-2", 4000, 3000, 300),
            ("2025-10-05", "019fa4ff-0002-7000-0000-000000000002", 1759665600i64, "grok-3", 5000, 4000, 500),
            ("2026-05-18", "019fa4ff-0003-7000-0000-000000000003", 1779105600i64, "grok-4.5-build", 8000, 6500, 800),
            ("2026-08-28", "019fa4ff-0004-7000-0000-000000000004", 1787983200i64, "grok-4.6-build", 10000, 8500, 1100),
        ];

        for (_date, sid, ts_sec, model, in_tok, cr_tok, out_tok) in sessions {
            let sess_dir = self.grok_dir.join("%2FUsers%2Ffelix%2FProjects%2Fai-lab").join(sid);
            fs::create_dir_all(&sess_dir).unwrap();

            let payload = serde_json::json!({
                "timestamp": ts_sec,
                "params": {
                    "sessionId": sid,
                    "update": {
                        "sessionUpdate": "turn_completed",
                        "usage": {
                            "inputTokens": in_tok,
                            "cachedReadTokens": cr_tok,
                            "outputTokens": out_tok,
                            "reasoningTokens": out_tok / 2,
                            "costUsdTicks": 15000000,
                            "modelUsage": {
                                model: {
                                    "inputTokens": in_tok,
                                    "cachedReadTokens": cr_tok,
                                    "outputTokens": out_tok,
                                    "reasoningTokens": out_tok / 2
                                }
                            }
                        }
                    }
                }
            });
            fs::write(sess_dir.join("updates.jsonl"), format!("{}\n", payload.to_string())).unwrap();
        }
    }

    fn populate_opencode_multi_year(&self) {
        let messages = [
            ("session-2024", "msg-101", 1716206400000i64, "claude-3-opus", 2000, 1000, 300, 100),
            ("session-2025", "msg-201", 1747828800000i64, "claude-3-5-sonnet", 3500, 2000, 500, 200),
            ("session-2026", "msg-301", 1787983200000i64, "claude-3-7-sonnet", 6000, 4000, 900, 300),
        ];

        for (sid, mid, ts_ms, model, in_tok, cr_tok, out_tok, cw_tok) in messages {
            let sess_dir = self.opencode_dir.join(sid);
            fs::create_dir_all(&sess_dir).unwrap();

            let content = serde_json::json!({
                "time": { "created": ts_ms, "completed": ts_ms + 3000 },
                "sessionID": sid,
                "path": { "root": "/Users/felix/Projects/compiler" },
                "model": { "modelID": model },
                "tokens": {
                    "input": in_tok,
                    "output": out_tok,
                    "reasoning": out_tok / 3,
                    "cache": {
                        "read": cr_tok,
                        "write": cw_tok
                    }
                }
            });
            fs::write(sess_dir.join(format!("{}.json", mid)), content.to_string()).unwrap();
        }
    }

    fn populate_history_db(&self) {
        let conn = rusqlite::Connection::open(self.config_dir.join("history.db")).unwrap();
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
            INSERT INTO daily_history VALUES ('2024-01-15', 50000, 5000, 30000, 4000, 89000, 1.25, '2026-08-28T00:00:00Z');
            INSERT INTO daily_history VALUES ('2024-06-20', 80000, 8000, 50000, 7000, 145000, 2.10, '2026-08-28T00:00:00Z');
            INSERT INTO daily_history VALUES ('2025-05-10', 120000, 15000, 90000, 12000, 237000, 3.50, '2026-08-28T00:00:00Z');
            INSERT INTO daily_history VALUES ('2025-11-25', 150000, 20000, 110000, 15000, 295000, 4.20, '2026-08-28T00:00:00Z');
            INSERT INTO daily_history VALUES ('2026-01-09', 200000, 25000, 150000, 20000, 395000, 5.80, '2026-08-28T00:00:00Z');",
        ).unwrap();

        // Also write history_overrides.json
        let overrides_content = serde_json::json!({
            "monthly_overrides": {
                "2024-01": {
                    "total_tokens": 100000,
                    "cost_usd": 1.50,
                    "input_tokens": 60000,
                    "output_tokens": 5000,
                    "cache_creation_input_tokens": 6000,
                    "cache_read_input_tokens": 35000
                }
            }
        });
        fs::write(self.config_dir.join("history_overrides.json"), overrides_content.to_string()).unwrap();
    }

    pub fn cli_cmd(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_tu"))
            .args(args)
            .args([
                "--claude-projects-dir",
                self.claude_dir.to_str().unwrap(),
                "--codex-sessions-dir",
                self.codex_dir.to_str().unwrap(),
                "--gemini-data-dir",
                self.gemini_dir.to_str().unwrap(),
                "--grok-log-dir",
                self.grok_dir.to_str().unwrap(),
                "--opencode-data-dir",
                self.opencode_dir.parent().unwrap().parent().unwrap().to_str().unwrap(),
            ])
            .env("HOME", &self.home_dir)
            .output()
            .unwrap()
    }
}

impl Drop for MultiYearFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

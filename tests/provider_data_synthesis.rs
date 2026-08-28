#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn make_test_env(test_name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("test-{}-{}", test_name, std::process::id()));
    let _ = fs::remove_dir_all(&root);

    let claude_dir = root.join("claude/projects");
    let codex_dir = root.join("codex/sessions");
    let gemini_dir = root.join("gemini/tmp");
    let grok_dir = root.join("grok/sessions");
    let opencode_dir = root.join("opencode/storage/message");

    fs::create_dir_all(&claude_dir).unwrap();
    fs::create_dir_all(&codex_dir).unwrap();
    fs::create_dir_all(&gemini_dir).unwrap();
    fs::create_dir_all(&grok_dir).unwrap();
    fs::create_dir_all(&opencode_dir).unwrap();

    (root, claude_dir, codex_dir, gemini_dir, grok_dir, opencode_dir)
}

fn write_claude_sample(claude_dir: &Path, project: &str, session_file: &str) {
    let proj_dir = claude_dir.join(format!("-Users-test-Projects-{}", project));
    fs::create_dir_all(&proj_dir).unwrap();
    let content = r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","timestamp":"2026-08-28T06:00:00.000Z","messageId":"msg_001","requestId":"req_001","message":{"model":"claude-3-7-sonnet-20250219","usage":{"input_tokens":1200,"cache_creation_input_tokens":500,"cache_read_input_tokens":800,"output_tokens":350,"reasoning_output_tokens":120}}}
"#;
    fs::write(proj_dir.join(session_file), content).unwrap();
}

fn write_codex_sample(codex_dir: &Path, date_folder: &str, session_file: &str) {
    let sess_dir = codex_dir.join(date_folder);
    fs::create_dir_all(&sess_dir).unwrap();
    let content = r#"{"type":"session_meta","payload":{"cwd":"/Users/test/Projects/codex-app"}}
{"type":"turn_context","payload":{"model":"gpt-5-codex"}}
{"type":"event_msg","timestamp":"2026-08-28T06:05:00.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":250,"reasoning_output_tokens":80,"total_tokens":1750}}}}
"#;
    fs::write(sess_dir.join(session_file), content).unwrap();
}

fn write_gemini_sample(gemini_dir: &Path, session_file: &str) {
    let content = r#"{"type":"gemini","timestamp":"2026-08-28T06:10:00.000Z","model":"gemini-3.6-flash","tokens":{"input":2000,"cached":1400,"output":400,"thoughts":100}}
"#;
    fs::write(gemini_dir.join(session_file), content).unwrap();
}

fn write_grok_sample(grok_dir: &Path, project_encoded: &str, session_id: &str) {
    let sess_dir = grok_dir.join(project_encoded).join(session_id);
    fs::create_dir_all(&sess_dir).unwrap();
    let content = format!(
        r#"{{"timestamp":1787983200,"params":{{"sessionId":"{}","update":{{"sessionUpdate":"turn_completed","usage":{{"inputTokens":6500,"cachedReadTokens":6000,"outputTokens":110,"reasoningTokens":65,"costUsdTicks":1948400,"modelUsage":{{"grok-4.6-build":{{"inputTokens":6500,"cachedReadTokens":6000,"outputTokens":110,"reasoningTokens":65}}}}}}}}}}}}
"#,
        session_id
    );
    fs::write(sess_dir.join("updates.jsonl"), content).unwrap();
}

fn write_opencode_sample(opencode_dir: &Path, session_id: &str, msg_id: &str) {
    let sess_dir = opencode_dir.join(session_id);
    fs::create_dir_all(&sess_dir).unwrap();
    let content = r#"{
  "time": {
    "created": 1787983200000,
    "completed": 1787983203000
  },
  "sessionID": "session-999",
  "path": {
    "root": "/Users/test/Projects/compiler"
  },
  "model": {
    "modelID": "claude-3-7-sonnet"
  },
  "tokens": {
    "input": 3000,
    "output": 600,
    "reasoning": 150,
    "cache": {
      "read": 2000,
      "write": 500
    }
  }
}"#;
    fs::write(sess_dir.join(format!("{}.json", msg_id)), content).unwrap();
}

#[test]
fn test_all_five_providers_synthesized_and_parsed_accurately() {
    let (root, claude_dir, codex_dir, gemini_dir, grok_dir, opencode_dir) =
        make_test_env("all-five-providers");

    write_claude_sample(&claude_dir, "web-app", "session-1.jsonl");
    write_codex_sample(&codex_dir, "2026-08-28", "session-1.jsonl");
    write_gemini_sample(&gemini_dir, "gemini-session.jsonl");
    write_grok_sample(
        &grok_dir,
        "%2FUsers%2Ftest%2FProjects%2Fmy-tool",
        "019fa4ff-8769-76c1-b662-ab0d9141d183",
    );
    write_opencode_sample(&opencode_dir, "session-999", "msg-001");

    let output = Command::new(env!("CARGO_BIN_EXE_tu"))
        .args([
            "daily",
            "--json",
            "--no-history-db",
            "--no-incremental-cache",
            "--claude-projects-dir",
            claude_dir.to_str().unwrap(),
            "--codex-sessions-dir",
            codex_dir.to_str().unwrap(),
            "--gemini-data-dir",
            gemini_dir.to_str().unwrap(),
            "--grok-log-dir",
            grok_dir.to_str().unwrap(),
            "--opencode-data-dir",
            opencode_dir.parent().unwrap().parent().unwrap().to_str().unwrap(),
        ])
        .env("HOME", root.join("home"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .unwrap_or_else(|e| panic!("Failed to parse JSON: {e}\nOutput was: {json_str}"));

    println!("Parsed JSON:\n{}", serde_json::to_string_pretty(&parsed).unwrap());

    let totals = &parsed["totals"];
    assert!(totals["total_tokens"].as_u64().unwrap() > 0);
    assert!(totals["input_tokens"].as_u64().unwrap() > 0);
    assert!(totals["output_tokens"].as_u64().unwrap() > 0);

    let daily = parsed["daily"].as_array().expect("daily should be an array");
    assert!(!daily.is_empty(), "daily array should not be empty");

    let mut all_sources = std::collections::BTreeSet::new();
    for day in daily {
        if let Some(sources) = day["sources"].as_object() {
            for key in sources.keys() {
                all_sources.insert(key.clone());
            }
        }
    }

    assert!(all_sources.contains("claude"), "missing claude source, sources found: {:?}", all_sources);
    assert!(all_sources.contains("codex"), "missing codex source, sources found: {:?}", all_sources);
    assert!(all_sources.contains("gemini"), "missing gemini source, sources found: {:?}", all_sources);
    assert!(all_sources.contains("grok"), "missing grok source, sources found: {:?}", all_sources);
    assert!(all_sources.contains("opencode"), "missing opencode source, sources found: {:?}", all_sources);

    let _ = fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
use std::io::{Read, Write};

#[cfg(target_os = "linux")]
#[test]
fn official_limits_only_is_a_no_history_codex_contract() {
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_tu"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("tu blocks --official-limits-only --json")
    );

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("official-limits-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let codex_home = root.join("codex");
    let cache_dir = root.join("cache/tokenusage");
    std::fs::create_dir_all(codex_home.join("sessions")).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();

    let legacy_cache = cache_dir.join("parse-cache-v2.json");
    let sentinel = b"history loader must not read or remove this cache";
    std::fs::write(&legacy_cache, sentinel).unwrap();
    let modified = std::fs::metadata(&legacy_cache)
        .unwrap()
        .modified()
        .unwrap();

    let run = |extra_args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_tu"))
            .args(["blocks", "--official-limits-only", "--json"])
            .args(extra_args)
            .env_clear()
            .env("HOME", root.join("home"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("CODEX_HOME", &codex_home)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap()
    };

    let disabled = run(&["--no-codex"]);
    assert!(!disabled.status.success());
    assert!(
        String::from_utf8_lossy(&disabled.stderr)
            .contains("--official-limits-only requires the Codex source")
    );

    let output = run(&[]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("oauth failed first"), "{stderr}");

    let missing_pricing = root.join("missing-pricing.json");
    let config = root.join("tu.json");
    std::fs::write(&config, br#"{"commands":{"blocks":{"live":true}}}"#).unwrap();
    std::fs::write(
        codex_home.join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-token"}"#,
    )
    .unwrap();

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            "chatgpt_base_url = \"http://{}\"\n",
            listener.local_addr().unwrap()
        ),
    )
    .unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.read(&mut [0; 1024]);
        let body = r#"{"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":12,"reset_at":123,"limit_window_seconds":18000},"secondary_window":{"used_percent":34,"reset_at":456,"limit_window_seconds":10080}}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let output = run(&[
        "--config",
        config.to_str().unwrap(),
        "--offline",
        "--pricing-file",
        missing_pricing.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({
            "official_codex": {
                "plan_type": "plus",
                "primary_used_percent": 12.0,
                "secondary_used_percent": 34.0,
                "primary_window_mins": 300,
                "secondary_window_mins": 168,
                "primary_resets_at": 123,
                "secondary_resets_at": 456
            }
        })
    );
    assert_eq!(std::fs::read(&legacy_cache).unwrap(), sentinel);
    assert_eq!(
        std::fs::metadata(&legacy_cache)
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert!(!cache_dir.join("parse-cache-v3.db").exists());
    let _ = std::fs::remove_dir_all(root);
}

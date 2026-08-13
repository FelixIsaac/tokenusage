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
    let home = root.join("home");
    std::fs::create_dir_all(codex_home.join("sessions")).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::create_dir_all(&home).unwrap();

    let legacy_cache = cache_dir.join("parse-cache-v2.json");
    let sentinel = b"history loader must not read or remove this cache";
    std::fs::write(&legacy_cache, sentinel).unwrap();
    let modified = std::fs::metadata(&legacy_cache)
        .unwrap()
        .modified()
        .unwrap();

    let disabled = std::process::Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["blocks", "--official-limits-only", "--json", "--no-codex"])
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("CODEX_HOME", &codex_home)
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(
        String::from_utf8_lossy(&disabled.stderr)
            .contains("--official-limits-only requires the Codex source")
    );

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tu"))
        .args(["blocks", "--official-limits-only", "--json"])
        .env_clear()
        .env("HOME", home)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("CODEX_HOME", codex_home)
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unexpected argument"), "{stderr}");
    assert!(stderr.contains("oauth failed first"), "{stderr}");
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

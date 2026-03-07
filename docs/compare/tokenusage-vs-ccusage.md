# tokenusage vs ccusage

**TL;DR:** tokenusage (`tu`) is a Rust-native alternative to ccusage that supports merged Codex + Claude + Antigravity tracking, runs 214x faster on warm cache, and works entirely locally.

## Feature Comparison

| Feature | tokenusage (`tu`) | ccusage |
|---|:---:|:---:|
| Claude Code log parsing | Yes | Yes |
| Codex log parsing | Yes | Yes (via `@ccusage/codex`) |
| Antigravity support | Yes | No |
| Merged multi-source dashboard | Yes | No (separate commands) |
| CLI report | Yes | Yes |
| Live TUI monitor | Yes (`tu live`) | No |
| htop-style session viewer | Yes (`tu top`) | No |
| Desktop GUI | Yes (`tu gui`) | No |
| Shareable image cards | Yes (`tu img`) | No |
| JSON output | Yes (`--json`) | Yes |
| jq integration | Yes (`--jq`) | No |
| Incremental cache | Yes | No |
| Offline mode | Yes (`--offline`) | No |
| Custom pricing file | Yes | No |
| Privacy (no log upload) | Yes | Yes |
| Language | Rust | TypeScript |
| Install size | ~15 MB binary | Node.js runtime + deps |

## Performance Benchmark

Machine: Apple M3 Max, macOS 15.6.1

### Claude logs — 1,521 JSONL files, 2.2 GB

| | `tu claude` | `bunx ccusage` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.73s** | 17.15s | **23.5x** |
| Warm (best of 5 / avg of 3) | **0.08s** | 17.15s | **214x** |

### Codex logs — 91 JSONL files, 1.7 GB

| | `tu codex` | `bunx @ccusage/codex` | Speedup |
|---|---:|---:|---:|
| Cold (rebuild cache) | **0.92s** | 20.76s | **22.6x** |
| Warm (best of 5 / avg of 3) | **0.15s** | 20.76s | **138x** |

### Why the difference?

1. **Rust-native parsing** — no JS runtime overhead, zero-copy JSONL deserialization
2. **Parallel file scanning** — uses rayon for parallel I/O across all CPU cores
3. **Incremental cache** — only re-parses files that changed since last run (by mtime + size)
4. **Single binary** — no dependency resolution or module loading at startup

## Reproduce the benchmark

```bash
# Install both tools
cargo install tokenusage --bin tu
npm install -g ccusage

# Warm cache run (Claude)
hyperfine --warmup 3 'tu claude' 'bunx ccusage' --min-runs 5

# Warm cache run (Codex)
hyperfine --warmup 3 'tu codex' 'bunx @ccusage/codex' --min-runs 5

# Cold cache run (tu only — clears cache first)
rm -rf ~/.cache/tokenusage && hyperfine --runs 1 'tu claude'
```

> Results will vary based on hardware, log volume, and filesystem cache state.

## When to use which

**Choose tokenusage if you:**
- Use both Codex and Claude Code and want one unified view
- Have large log directories and need sub-second response
- Want a live TUI, GUI, or shareable image cards
- Prefer a single binary with no runtime dependencies

**Choose ccusage if you:**
- Only use Claude Code and prefer a Node.js ecosystem tool
- Are already integrated with ccusage in your workflow
- Need the specific ccusage output format for existing scripts

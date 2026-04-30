# Content Templates

Ready-to-use templates for each platform. Copy, customize, and post.

---

## Twitter/X

### A1 — Launch post

```
Built a fast local token tracker for Codex + Claude Code.

One command: `tu`
Result: your daily AI coding costs in 0.08s

- merged Codex + Claude + Antigravity view
- CLI / live TUI / GUI / shareable image cards
- 214x faster than ccusage (benchmark in repo)
- all local, no log upload

github.com/hanbu97/tokenusage
```

### A2 — Benchmark post

```
I benchmarked tokenusage against ccusage on real local logs.

Warm-cache results:
- 138x faster on Codex logs (0.15s vs 20.76s)
- 214x faster on Claude logs (0.08s vs 17.15s)

Rust-native scan + parallel I/O + incremental cache.

Reproducible: `hyperfine --warmup 3 'tu claude' 'bunx ccusage'`

github.com/hanbu97/tokenusage
```

### A3 — tu top (htop angle)

```
I built htop for AI tokens.

`tu top` shows active sessions across providers in real-time:
- token counts per session
- cost per session
- model breakdown
- sorted by activity

cargo install tokenusage --bin tu && tu top
```

### A4 — tu img (shareable cards)

```
Want to know your AI coding costs?

Run `tu img` and it generates a shareable card with:
- hourly token breakdown
- cost chart
- model distribution

Post yours with #MyAICodingCost

npm i -g tokenusage && tu img

github.com/hanbu97/tokenusage
```

---

## Reddit

### B1 — r/ClaudeAI

```
Title: How I track my Claude Code costs locally in 0.08s (with benchmarks)

I've been using Claude Code heavily and wanted a fast way to see my daily/weekly
token usage without uploading anything.

Built `tokenusage` (`tu`) in Rust. Scans local JSONL logs, gives you:
- Daily/weekly/monthly breakdown
- Live TUI monitor (`tu live`)
- htop-style session viewer (`tu top`)
- Shareable image cards (`tu img`)
- Also supports Codex, Gemini, OpenCode, and Antigravity in one unified view

Performance: 1,521 files (2.2 GB), warm cache = 0.08s (vs ccusage 17.15s)

Install: npm i -g tokenusage
Repo: github.com/hanbu97/tokenusage

Happy to answer questions or take feature requests.
```

### B2 — r/rust

```
Title: tokenusage — fast local AI coding token tracker (CLI/TUI/GUI)

Sharing a Rust project I've been working on. `tokenusage` (`tu`) scans local
AI coding session logs and gives usage/cost reports.

Technical highlights:
- rayon for parallel JSONL parsing across ~1500 files
- Incremental cache (mtime + size check, skip unchanged files)
- 0.08s warm-cache on 2.2 GB of Claude logs
- ratatui for live TUI, iced for desktop GUI
- plotters + image crate for shareable PNG cards

Repo: github.com/hanbu97/tokenusage

Would love feedback on the Rust side — especially around the caching strategy
and parallel I/O patterns.
```

### B3 — r/ChatGPTCoding

```
Title: Track your Codex costs with one command

Built a local CLI that scans Codex session logs and shows daily token usage + cost.

One command: `tu codex`

Also supports Claude Code, Gemini CLI, OpenCode, and Antigravity in the same dashboard.

Features: CLI report, live TUI, htop-style session viewer, GUI, image cards.

npm i -g tokenusage

github.com/hanbu97/tokenusage
```

### B4 — r/codex

```
Title: One-command Codex usage dashboard

If you're using Codex and want to see daily/weekly token usage and cost:

`tu codex` — scans ~/.codex/sessions, gives a table in 0.15s

Also has:
- `tu top` — htop for AI tokens (live per-session view)
- `tu live` — real-time TUI monitor
- `tu img` — shareable usage card

Works with Claude Code, Gemini CLI, and OpenCode too, merged into one dashboard.

npm i -g tokenusage
github.com/hanbu97/tokenusage
```

---

## V2EX

### C1 — Launch post

```
Title: 用 Rust 写了个多数据源 AI token 追踪 CLI，0.08 秒出结果

日常同时用 Codex、Claude Code、Gemini CLI 和 OpenCode，想知道每天花了多少 token 和钱。
写了 tokenusage（命令 tu），Rust 并行扫描 + 增量缓存。

主要功能: tu / tu live / tu top / tu img / tu gui
安装: npm i -g tokenusage
GitHub: github.com/hanbu97/tokenusage

欢迎试用和反馈。
```

### C2 — tu top follow-up

```
Title: tu top: 像 htop 一样监控 AI token 使用

上次分享了 tokenusage，最近加了 `tu top` 功能。

实时显示各 provider 会话的:
- token 数量
- 费用
- 模型分布
- 按活跃度排序

安装: npm i -g tokenusage && tu top
GitHub: github.com/hanbu97/tokenusage
```

---

## LinkedIn

### D1 — Technical story

```
I built a CLI that tracks AI coding costs 214x faster than existing tools.

I use Codex, Claude Code, Gemini CLI, and OpenCode daily. Multiple tools and log directories, no unified view.

Existing solutions were either slow (17+ seconds), cloud-only, or single-source.

So I built tokenusage in Rust:
- 0.08s warm-cache on 2.2 GB of logs
- One dashboard: Codex + Claude + Gemini + OpenCode + Antigravity
- Privacy-first: all local

Latest: `tu top` — htop for AI tokens.

github.com/hanbu97/tokenusage

What tools do you use to track your AI coding costs?
```

---

## Hacker News

### Show HN

```
Title: Show HN: tokenusage – Track AI coding costs locally, 214x faster (Rust)

Body:
I built tokenusage because I wanted one local view of token and cost usage
across Codex, Claude Code, Gemini CLI, and OpenCode without uploading logs anywhere.

The tool scans local JSONL session logs and provides:
- daily/weekly/monthly CLI report
- live TUI monitor (`tu live`)
- htop-style per-session viewer (`tu top`)
- desktop GUI (`tu gui`)
- shareable image cards (`tu img`)

Performance was a priority. On my local datasets (1,521 files, 2.2 GB):
- Warm cache: 0.08s (vs ccusage 17.15s = 214x faster)
- Cold cache: 0.73s (vs 17.15s = 23.5x faster)

This comes from Rust-native parsing, rayon parallelism, and incremental caching.

Install: npm i -g tokenusage (or cargo install tokenusage --bin tu)
Repo: https://github.com/hanbu97/tokenusage

Happy to discuss the technical approach or take feature requests.
```

Timing: Post on a weekday (Monday or Tuesday), US Pacific 8-9 AM.
After posting: share the HN link on Twitter, reply to every comment.

---

## Rust Forum (Announcements)

```
Title: tokenusage: fast local Codex + Claude Code token tracker (CLI/TUI/GUI)

Body:
I released tokenusage, a Rust CLI for tracking AI coding token usage and costs
from local session logs.

Key features:
- Merged Codex + Claude Code + Antigravity dashboard
- CLI report, live TUI (ratatui), desktop GUI (iced)
- Shareable PNG image cards (plotters + image)
- 214x faster than ccusage on warm cache (parallel scan + incremental cache)

Install: cargo install tokenusage --bin tu
Repo: https://github.com/hanbu97/tokenusage

Feedback on caching strategy, CLI ergonomics, or Rust patterns welcome.
```

---

## Hook Formula Library

Use these as opening lines to increase engagement:

| Type | Hook |
|---|---|
| Curiosity | "I was spending $47/day on AI coding and didn't know it." |
| Curiosity | "214x is not a typo. Here's the benchmark." |
| Story | "Last week my Claude Code hit the rate limit mid-refactor." |
| Value | "How to know your exact AI coding costs in 0.08 seconds:" |
| Contrarian | "I stopped using ccusage and built something 214x faster." |
| Challenge | "Show me your AI coding costs. Run `tu img`. Reply with your card." |

---

## Content Pillars

| Pillar | Share | Example topics |
|---|---|---|
| Performance / data insights | 35% | "214x faster: how and why", benchmark deep-dive |
| Tutorials / how-to | 30% | "tu top: htop for tokens", "tu img: share your stats" |
| Developer story | 20% | "Why I chose Rust for a CLI", "building a TUI in ratatui" |
| Release updates | 10% | "v1.4.0: what's new" |
| User-generated content | 5% | RT user `tu img` shares, #MyAICodingCost |

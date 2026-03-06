# tokenusage Star Growth Playbook

Date: 2026-03-06 (Asia/Shanghai)
Target: grow `https://github.com/hanbu97/tokenusage` from single-digit stars to 100

## Baseline

- GitHub stars: 4
- Forks: 0
- Watchers: 0
- Open issues: 0

This is still an awareness problem, not a product problem.

## What Will Actually Move Stars

1. Better first-screen conversion on the repo page.
2. Repeated distribution into channels where developers already compare tools.
3. Shareable assets that make people repost without extra work.
4. Clear proof that `tokenusage` is faster and supports merged Codex + Claude usage.

## Immediate Repo-Side Conversion Checklist

1. Keep the README hero focused on one sentence, one proof point, and one star CTA.
2. Use the screenshot grid to show four entry points fast:
   - `tu`
   - `tu gui`
   - `tu live`
   - `tu img`
3. Keep the install block above the long explanation.
4. Keep benchmark numbers near the top.
5. Keep package descriptions consistent across GitHub, crates.io, npm, and PyPI.
6. Set a GitHub social preview image manually in repo settings.
7. Add a short website/homepage manually in GitHub repo settings if you later host docs.

## Channel Priority

### Tier 1

1. X / Twitter
2. Hacker News (`Show HN`)
3. Rust Forum (`Announcements`)
4. Relevant GitHub awesome-list PRs

### Tier 2

1. Reddit posts that are framed as benchmarks or workflow writeups, not ads
2. V2EX / 掘金 / 少数派 style developer posts
3. Short demo video posts using the same visuals as `tu img`

## Post Angles That Fit tokenusage

1. "I wanted one local usage tracker for Codex + Claude, so I wrote one in Rust."
2. "ccusage is great, but I needed something much faster on large local logs."
3. "I added shareable image cards so AI coding usage can be posted like a stat card."
4. "Real-time Codex/Claude usage in CLI, TUI, and GUI without uploading logs."

## Asset Mapping

Use these assets repeatedly instead of making new ones every time:

1. CLI proof: `docs/images/cli-demo-padded.png`
2. GUI proof: `docs/images/gui-demo.png`
3. Live proof: `docs/images/live-demo.png`
4. Share card daily: `docs/images/share-demo.png`
5. Share card weekly: `docs/images/share-week-demo.png`

## Ready-To-Post Copy

### X / Twitter launch post

```text
Built a fast local token usage tracker for AI coding workflows:

- merged Codex + Claude usage
- CLI / live TUI / GUI
- shareable daily + weekly stat cards
- much faster than ccusage on my local logs

Repo: https://github.com/hanbu97/tokenusage

If you use Codex or Claude Code, try:
cargo install tokenusage --bin tu
```

### X / Twitter benchmark post

```text
I benchmarked tokenusage against ccusage on real local logs.

Warm-cache results on my machine:
- 138x faster on Codex logs
- 214x faster on Claude logs

Main reason: Rust-native scan/parsing + aggressive cache reuse.

Repo: https://github.com/hanbu97/tokenusage
```

### Hacker News `Show HN`

Title:

```text
Show HN: tokenusage – a fast local Codex + Claude usage tracker in Rust
```

Body:

```text
I built tokenusage because I wanted one local view of token and cost usage across Codex and Claude Code.

The tool scans local logs, merges the results, and exposes:
- a daily/weekly/monthly CLI report
- a live TUI monitor
- a desktop GUI
- shareable image cards for daily and weekly usage

I also focused heavily on speed. On my local datasets it is much faster than ccusage, especially on warm cache runs.

Repo: https://github.com/hanbu97/tokenusage
```

### Rust Forum `Announcements`

Title:

```text
tokenusage: fast local Codex + Claude usage tracker in Rust
```

Body:

```text
I released tokenusage, a Rust tool for tracking AI coding usage from local session logs.

Core points:
- merged Codex + Claude usage
- CLI, live TUI, GUI
- local parsing, no log upload
- shareable image cards via `tu img`
- benchmark-driven performance focus

Repository:
https://github.com/hanbu97/tokenusage

I would especially value feedback on data sources, pricing accuracy, and CLI/TUI ergonomics.
```

### Reddit-style technical post

```text
I needed one local usage tracker for Codex + Claude, and I wanted it to stay fast on big log directories, so I built tokenusage in Rust.

The part I cared about most was keeping the common path fast:
- parallel file discovery
- cached parsing
- merged daily/weekly/monthly reports
- live monitor
- image-card export

It currently supports CLI, TUI, and GUI.

Repo: https://github.com/hanbu97/tokenusage
```

## Awesome-List Targets

These are good fits because they are discovery surfaces, not short-lived feeds.

1. `github.com/jtsang4/awesome-claude-code`
2. `github.com/sourcegraph/awesome-code-ai`
3. `github.com/ai-for-developers/awesome-ai-devtools`
4. Rust CLI / terminal tooling lists that accept analytics or productivity tools

When opening a PR:

1. Keep the description factual.
2. Use one sentence only.
3. Mention merged Codex + Claude tracking and Rust speed.

Suggested list sentence:

```text
tokenusage — fast local token/cost tracker for Codex and Claude Code with CLI, live TUI, GUI, and shareable usage cards.
```

## Weekly Execution Loop

### Monday

1. Post one benchmark or usage insight.
2. Open one awesome-list PR.

### Wednesday

1. Post one workflow clip or screenshot.
2. Ship one small repo improvement that is easy to show.

### Friday

1. Post one release or feature thread.
2. Re-share the best visual (`tu img` or `tu live`) with a different hook.

## Simple Success Metric

Track only these every week:

1. GitHub stars
2. npm weekly downloads
3. crates.io total downloads

If a channel drives clicks but not stars, improve the repo page.
If the repo page converts but reach is low, post more often.

## What Not To Do

1. Do not spam the same copy across multiple communities on the same day.
2. Do not lead with "please star" before showing proof.
3. Do not post generic feature lists without one strong visual or benchmark.
4. Do not dilute the message. Lead with Codex + Claude merged tracking and speed.

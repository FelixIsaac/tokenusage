# tokenusage Weekly Growth Tracker

## How to update

Every Sunday, fill in the current week's row. Track stars, npm weekly downloads, and crates.io total downloads.

Check commands:
```bash
# GitHub stars (requires gh CLI)
gh api repos/hanbu97/tokenusage --jq '.stargazers_count'

# npm weekly downloads
curl -s "https://api.npmjs.org/downloads/point/last-week/tokenusage" | python3 -c "import sys,json; print(json.load(sys.stdin)['downloads'])"

# crates.io total downloads
curl -s "https://crates.io/api/v1/crates/tokenusage" | python3 -c "import sys,json; print(json.load(sys.stdin)['crate']['downloads'])"
```

## Weekly Data

| Week ending | Stars | npm weekly DL | crates.io total DL | Best post | Worst post | Notes |
|---|---:|---:|---:|---|---|---|
| 2026-03-09 | 4 | - | 109 | - | - | Baseline |
| 2026-03-16 | | | | | | |
| 2026-03-23 | | | | | | |
| 2026-03-30 | | | | | | |
| 2026-04-06 | | | | | | |
| 2026-04-13 | | | | | | |
| 2026-04-20 | | | | | | |
| 2026-04-27 | | | | | | |

## KPI Targets

| Metric | Baseline | Week 4 target | Week 8 target |
|---|---:|---:|---:|
| GitHub Stars | 4 | 15-30 | 60-100 |
| npm weekly DL | 859 | 1,200 | 1,500+ |
| crates.io total DL | 109 | 200 | 300+ |

## Channel ROI (update at week 4 and 8)

| Channel | Posts | Estimated stars driven | Hours spent | Stars/hour | Keep? |
|---|---:|---:|---:|---:|---|
| Twitter/X | | | | | |
| Reddit | | | | | |
| Hacker News | | | | | |
| V2EX | | | | | |
| Awesome-list PRs | | | | | |
| Rust Forum | | | | | |
| LinkedIn | | | | | |

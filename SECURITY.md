# Security notes

This project consumes local logs/datastores produced by various AI coding tools. It **does not**
require network access for core usage aggregation, but some commands may fetch:

- model pricing (OpenRouter model list / pricing)
- provider "official" limits (where supported)

## Quick checks

- Dependency advisories: `cargo audit`
- Build from source: `cargo build --release`
- Prefer running `tu doctor` to confirm:
  - which local roots were scanned
  - how many files/events were discovered
  - pricing cache age/TTL (to understand cost diffs vs other tools)

## Cargo audit status

`cargo audit` may report warnings such as:

- **unmaintained** dependencies
- **yanked** crates
- **unsound** advisories

These are not always exploitable vulnerabilities, but they are worth tracking, especially for UI
dependencies (`iced`, `ratatui`, `wgpu`) pulled in by optional TUI/GUI features.


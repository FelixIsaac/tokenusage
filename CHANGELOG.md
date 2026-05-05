# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Added
- Report insights pipeline for daily/weekly/monthly outputs, including cache/output share, efficiency metrics, peak periods, streaks, spikes, anomalies, and provider mix.
- GUI report caching for faster switching between `daily`, `weekly`, and `monthly` views.
- GUI insights summary cards for top source, top model, peak period, and anomaly.
- `today` supports explicit `--since` / `--until` ranges.

### Changed
- CLI `Insights:` summary now includes spike/anomaly attribution with top source/model.
- TOTAL-row model summary now reports provider-aware model counts.

### Fixed
- OpenCode ingestion improved for DB + legacy merge behavior and session id stability.
- `tu gui` startup panic on Windows async runtime teardown.


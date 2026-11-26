use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, bounded};
use ignore::WalkBuilder;
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

use crate::cli::CommonArgs;
use crate::types::{
    CodexParseState, CodexRawUsage, DateFilter, DiscoveredFile, LEGACY_CODEX_FALLBACK_MODEL,
    ParseLineResult, ParseStatsAtomic, ParsedLine, PricingContext, PricingRate, PricingTable,
    ServiceSpeed, SourceConfig, SourceKind, UsageAccumulator, UsageEvent,
};

use super::cache_codec::{decode_cached_file_entry, encode_cached_file_entry, entry_ts_bounds};
use super::pricing::*;
use super::*;

#[inline]
pub(super) fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn filter_utc_window_ms(filter: DateFilter) -> (Option<i64>, Option<i64>) {
    // Expand by +/- 24 hours so every IANA timezone (UTC-12..UTC+14) is covered safely.
    const PAD_MS: i64 = 24 * 3_600_000;
    let since_ms = filter.since.and_then(|d| {
        d.and_hms_opt(0, 0, 0)
            .map(|dt| dt.and_utc().timestamp_millis().saturating_sub(PAD_MS))
    });
    let until_ms = filter.until.and_then(|d| {
        d.and_hms_opt(23, 59, 59)
            .map(|dt| dt.and_utc().timestamp_millis().saturating_add(PAD_MS))
    });
    (since_ms, until_ms)
}

pub(super) struct ParseFilesConfig<'a> {
    pub filter: DateFilter,
    pub timezone: &'a TimeZoneMode,
    pub pricing: Arc<PricingTable>,
    pub worker_count: usize,
    pub cache_enabled: bool,
    pub cache_path: Option<&'a Path>,
    pub sort_events: bool,
}

pub(super) fn parse_files_with_cache(
    files: &[DiscoveredFile],
    cache_store: &mut IncrementalCacheStore,
    config: ParseFilesConfig<'_>,
) -> ParsedUsageOutput {
    let ParseFilesConfig {
        filter,
        timezone,
        pricing,
        worker_count,
        cache_enabled,
        cache_path,
        sort_events,
    } = config;
    let stats = Arc::new(ParseStatsAtomic::default());
    stats.files_discovered.store(files.len(), Ordering::Relaxed);

    let mut cache_dirty = false;
    let (since_ms, until_ms) = filter_utc_window_ms(filter);

    enum PassResult {
        Hit(Vec<InternalEvent>),
        FetchBlobHit(usize, String, CachedFileMeta),
        FetchBlobIncr(usize, String, FileFingerprint, CachedFileMeta),
        Job(Box<FileParseJob>),
        SkippedOld,
    }

    let use_meta = cache_enabled && cache_store.meta_only && cache_path.is_some();
    let pre_update_meta_len = cache_store.meta.len();
    let matched_meta_keys = std::sync::atomic::AtomicUsize::new(0);

    let pass: Vec<PassResult> = files
        .par_iter()
        .enumerate()
        .map(|(idx, file)| {
            let key = if cache_enabled {
                cache_file_key(&file.path)
            } else {
                String::new()
            };
            let fingerprint = cache_enabled
                .then(|| read_file_fingerprint(&file.path))
                .flatten();
            let Some(fingerprint) = fingerprint else {
                if use_meta && cache_store.meta.contains_key(&key) {
                    matched_meta_keys.fetch_add(1, Ordering::Relaxed);
                }
                return PassResult::Job(Box::new(FileParseJob {
                    file: file.clone(),
                    cache_key: key,
                    fingerprint: FileFingerprint {
                        size: 0,
                        modified_unix_secs: 0,
                        modified_unix_nanos: 0,
                    },
                    strategy: ParseStrategy::Full,
                }));
            };

            if use_meta {
                if let Some(meta) = cache_store.meta.get(&key) {
                    matched_meta_keys.fetch_add(1, Ordering::Relaxed);
                    if meta.fingerprint == fingerprint {
                        let out_of_window = (meta.min_ts_ms == 0 && meta.max_ts_ms == 0)
                            || since_ms.is_some_and(|s| meta.max_ts_ms > 0 && meta.max_ts_ms < s)
                            || until_ms.is_some_and(|u| meta.min_ts_ms > 0 && meta.min_ts_ms > u);
                        if out_of_window {
                            stats
                                .lines_total
                                .fetch_add(meta.stats.lines_total, Ordering::Relaxed);
                            stats
                                .lines_invalid_json
                                .fetch_add(meta.stats.lines_invalid_json, Ordering::Relaxed);
                            stats
                                .lines_missing_usage
                                .fetch_add(meta.stats.lines_missing_usage, Ordering::Relaxed);
                            return PassResult::SkippedOld;
                        }
                        return PassResult::FetchBlobHit(idx, key, meta.clone());
                    }
                    if can_incremental_parse_meta(meta, fingerprint) {
                        return PassResult::FetchBlobIncr(idx, key, fingerprint, meta.clone());
                    }
                }
                if let Some(s) = since_ms
                    && fingerprint.modified_unix_secs.saturating_mul(1000)
                        < s.saturating_sub(48 * 3_600_000)
                {
                    return PassResult::SkippedOld;
                }
                return PassResult::Job(Box::new(FileParseJob {
                    file: file.clone(),
                    cache_key: key,
                    fingerprint,
                    strategy: ParseStrategy::Full,
                }));
            }

            if let Some(cached) = cache_store.files.get(&key) {
                if cached.fingerprint == fingerprint {
                    return PassResult::Hit(hydrate_cached_internal_events(
                        file, cached, filter, timezone, &pricing, &stats,
                    ));
                }
                if can_incremental_parse(cached, fingerprint) {
                    return PassResult::Job(Box::new(FileParseJob {
                        file: file.clone(),
                        cache_key: key,
                        fingerprint,
                        strategy: ParseStrategy::Incremental {
                            base_cache: cached.clone(),
                        },
                    }));
                }
            }

            if let Some(s) = since_ms
                && fingerprint.modified_unix_secs.saturating_mul(1000)
                    < s.saturating_sub(48 * 3_600_000)
            {
                return PassResult::SkippedOld;
            }

            PassResult::Job(Box::new(FileParseJob {
                file: file.clone(),
                cache_key: key,
                fingerprint,
                strategy: ParseStrategy::Full,
            }))
        })
        .collect();

    let mut parse_jobs = Vec::new();
    let mut internal_events = Vec::new();
    let mut fetch_hits = Vec::new();
    let mut fetch_incr = Vec::new();

    for result in pass {
        match result {
            PassResult::Hit(hit) => internal_events.extend(hit),
            PassResult::FetchBlobHit(idx, key, meta) => fetch_hits.push((idx, key, meta)),
            PassResult::FetchBlobIncr(idx, key, fp, meta) => fetch_incr.push((idx, key, fp, meta)),
            PassResult::Job(job) => parse_jobs.push(*job),
            PassResult::SkippedOld => {}
        }
    }

    if (!fetch_hits.is_empty() || !fetch_incr.is_empty())
        && let Some(path) = cache_path
        && let Ok(conn) = open_cache_db_readonly(path)
    {
        let mut raw_hits: Vec<(usize, CachedFileMeta, Vec<u8>)> =
            Vec::with_capacity(fetch_hits.len());
        if fetch_hits.len() + fetch_incr.len() > 512 {
            let mut hit_map: HashMap<String, (usize, CachedFileMeta)> =
                HashMap::with_capacity(fetch_hits.len());
            for (idx, key, meta) in fetch_hits {
                hit_map.insert(key, (idx, meta));
            }
            let mut incr_map: HashMap<String, (usize, FileFingerprint, CachedFileMeta)> =
                HashMap::with_capacity(fetch_incr.len());
            for (idx, key, fp, meta) in fetch_incr {
                incr_map.insert(key, (idx, fp, meta));
            }
            let sql = match (since_ms, until_ms) {
                (Some(_), Some(_)) => {
                    "SELECT cache_key, blob FROM files_v5 WHERE (max_ts_ms <= 0 OR max_ts_ms >= ?1) AND (min_ts_ms <= 0 OR min_ts_ms <= ?2)"
                }
                (Some(_), None) => {
                    "SELECT cache_key, blob FROM files_v5 WHERE max_ts_ms <= 0 OR max_ts_ms >= ?1"
                }
                (None, Some(_)) => {
                    "SELECT cache_key, blob FROM files_v5 WHERE min_ts_ms <= 0 OR min_ts_ms <= ?1"
                }
                (None, None) => "SELECT cache_key, blob FROM files_v5",
            };
            if let Ok(mut stmt) = conn.prepare(sql) {
                let rows_res = match (since_ms, until_ms) {
                    (Some(s), Some(u)) => stmt.query(params![s, u]),
                    (Some(s), None) => stmt.query(params![s]),
                    (None, Some(u)) => stmt.query(params![u]),
                    (None, None) => stmt.query([]),
                };
                if let Ok(mut rows) = rows_res {
                    while let Ok(Some(row)) = rows.next() {
                        let Ok(key) = row.get::<_, String>(0) else {
                            continue;
                        };
                        if let Some((idx, meta)) = hit_map.remove(&key) {
                            if let Ok(blob) = row.get::<_, Vec<u8>>(1) {
                                raw_hits.push((idx, meta, blob));
                            } else {
                                parse_jobs.push(FileParseJob {
                                    file: files[idx].clone(),
                                    cache_key: key,
                                    fingerprint: meta.fingerprint,
                                    strategy: ParseStrategy::Full,
                                });
                            }
                        } else if let Some((idx, fp, meta)) = incr_map.remove(&key) {
                            if let Ok(blob) = row.get::<_, Vec<u8>>(1)
                                && let Ok(entry) = decode_cached_file_entry(
                                    &blob,
                                    meta.fingerprint,
                                    meta.stats.clone(),
                                    meta.parsed_offset,
                                )
                            {
                                parse_jobs.push(FileParseJob {
                                    file: files[idx].clone(),
                                    cache_key: key,
                                    fingerprint: fp,
                                    strategy: ParseStrategy::Incremental { base_cache: entry },
                                });
                            } else {
                                parse_jobs.push(FileParseJob {
                                    file: files[idx].clone(),
                                    cache_key: key,
                                    fingerprint: fp,
                                    strategy: ParseStrategy::Full,
                                });
                            }
                        }
                    }
                }
            }
            for (key, (idx, meta)) in hit_map {
                parse_jobs.push(FileParseJob {
                    file: files[idx].clone(),
                    cache_key: key,
                    fingerprint: meta.fingerprint,
                    strategy: ParseStrategy::Full,
                });
            }
            for (key, (idx, fp, _meta)) in incr_map {
                parse_jobs.push(FileParseJob {
                    file: files[idx].clone(),
                    cache_key: key,
                    fingerprint: fp,
                    strategy: ParseStrategy::Full,
                });
            }
        } else if let Ok(mut stmt) = conn.prepare("SELECT blob FROM files_v5 WHERE cache_key = ?1")
        {
            for (idx, key, meta) in fetch_hits {
                if let Ok(blob) = stmt.query_row(params![&key], |r| r.get::<_, Vec<u8>>(0)) {
                    raw_hits.push((idx, meta, blob));
                } else {
                    parse_jobs.push(FileParseJob {
                        file: files[idx].clone(),
                        cache_key: key,
                        fingerprint: meta.fingerprint,
                        strategy: ParseStrategy::Full,
                    });
                }
            }
            for (idx, key, fp, meta) in fetch_incr {
                if let Ok(blob) = stmt.query_row(params![&key], |r| r.get::<_, Vec<u8>>(0))
                    && let Ok(entry) = decode_cached_file_entry(
                        &blob,
                        meta.fingerprint,
                        meta.stats.clone(),
                        meta.parsed_offset,
                    )
                {
                    parse_jobs.push(FileParseJob {
                        file: files[idx].clone(),
                        cache_key: key,
                        fingerprint: fp,
                        strategy: ParseStrategy::Incremental { base_cache: entry },
                    });
                } else {
                    parse_jobs.push(FileParseJob {
                        file: files[idx].clone(),
                        cache_key: key,
                        fingerprint: fp,
                        strategy: ParseStrategy::Full,
                    });
                }
            }
        }

        let decoded_hits: Vec<Vec<InternalEvent>> = raw_hits
            .par_iter()
            .map(|(idx, meta, blob)| {
                match decode_cached_file_entry(
                    blob,
                    meta.fingerprint,
                    meta.stats.clone(),
                    meta.parsed_offset,
                ) {
                    Ok(entry) => hydrate_cached_internal_events_owned(
                        &files[*idx],
                        entry,
                        filter,
                        timezone,
                        &pricing,
                        &stats,
                    ),
                    Err(_) => Vec::new(),
                }
            })
            .collect();
        for hit in decoded_hits {
            internal_events.extend(hit);
        }
    }

    let parsed = parse_files_concurrently(
        parse_jobs,
        worker_count.max(1),
        filter,
        timezone.clone(),
        pricing.clone(),
        stats.clone(),
        cache_enabled,
    );
    internal_events.extend(parsed.events);

    if cache_enabled {
        for (key, entry) in parsed.cache_updates {
            let (min_ts_ms, max_ts_ms) = entry_ts_bounds(&entry);
            cache_store.meta.insert(
                key.clone(),
                CachedFileMeta {
                    fingerprint: entry.fingerprint,
                    min_ts_ms,
                    max_ts_ms,
                    parsed_offset: entry.parsed_offset,
                    stats: entry.stats.clone(),
                },
            );
            cache_store.changed.insert(key.clone());
            cache_store.files.insert(key, entry);
            cache_dirty = true;
        }

        // Skip eviction when every pre-existing meta key matched in `files` (no deletions possible).
        let all_meta_present =
            use_meta && matched_meta_keys.load(Ordering::Relaxed) == pre_update_meta_len;
        if !all_meta_present {
            // Evict entries for files no longer present — but ONLY under roots this
            // run actually scanned.
            let seen_cache_keys: HashSet<String> = files
                .iter()
                .map(|file| cache_file_key(&file.path))
                .collect();
            let unique_roots: HashSet<&Path> =
                files.iter().map(|file| file.root.as_path()).collect();
            let scanned_root_keys: Vec<String> =
                unique_roots.into_iter().map(cache_file_key).collect();
            let removed_keys: Vec<String> = cache_store
                .meta
                .keys()
                .chain(cache_store.files.keys())
                .filter(|key| {
                    !seen_cache_keys.contains(key.as_str())
                        && scanned_root_keys
                            .iter()
                            .any(|root| key.starts_with(root.as_str()))
                })
                .cloned()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            for key in removed_keys {
                cache_store.files.remove(&key);
                cache_store.meta.remove(&key);
                cache_store.changed.remove(&key);
                cache_store.removed.insert(key);
                cache_dirty = true;
            }
        }
    }

    // Cross-file deduplication for Claude (handles resumed/forked transcripts deterministically
    // using max() per token channel) and Grok (if prompt copies exist across files).
    if files.iter().any(|file| file.source == SourceKind::Claude) {
        dedupe_claude_events_max_merge(&mut internal_events, &pricing);
    }
    if files.iter().any(|file| {
        matches!(
            file.source,
            SourceKind::Grok | SourceKind::Pi | SourceKind::Muse
        )
    }) {
        dedupe_hash_events(&mut internal_events);
    }

    let mut events: Vec<UsageEvent> = internal_events.into_iter().map(|item| item.event).collect();

    if sort_events {
        events.par_sort_unstable_by_key(|e| e.timestamp);
    }
    if files.iter().any(|file| file.source == SourceKind::OpenCode) {
        dedupe_opencode_events(&mut events);
    }

    ParsedUsageOutput {
        loaded: LoadedUsage {
            events,
            stats: stats.snapshot(),
            filter,
            pricing,
        },
        cache_dirty,
    }
}

impl LiveUsageRuntime {
    pub(super) async fn new(
        common: &CommonArgs,
        refresh_every: u64,
        defer_claude: bool,
    ) -> Result<Self> {
        let filter = parse_common_filter(common)?;
        let sources = build_sources(common).await?;
        if sources.is_empty() {
            bail!(
                "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
            );
        }

        // When deferring Claude, use cached-only pricing (no network wait) so
        // the first frame renders instantly.  The stale/missing pricing will be
        // refreshed on the next full load cycle via maybe_refresh_sources.
        let pricing = if defer_claude {
            Arc::new(load_pricing(common.pricing_file.as_deref(), true).await?)
        } else {
            Arc::new(load_pricing(common.pricing_file.as_deref(), common.offline).await?)
        };
        let pricing_key = pricing_cache_key(&pricing);
        let ignore_rules = PathIgnoreRules::from_common(common);
        let worker_count = worker_count_from_common(common);

        let cache_enabled = !common.no_incremental_cache;
        let cache_path = incremental_cache_path();
        let mut cache_store = if cache_enabled {
            match cache_path.as_ref() {
                Some(path) => load_incremental_cache(path, &pricing_key),
                None => IncrementalCacheStore::new(pricing_key.clone()),
            }
        } else {
            IncrementalCacheStore::new(pricing_key.clone())
        };
        if common.rebuild_cache {
            cache_store = IncrementalCacheStore::new(pricing_key);
        }

        // When defer_claude is set, only discover non-Claude sources so the
        // first frame renders instantly.  Claude directory walking + parsing
        // is deferred to the second load cycle.
        let (files_cache, deferred_claude_files) = if defer_claude {
            let fast_sources: Vec<_> = sources
                .iter()
                .filter(|s| s.kind != SourceKind::Claude)
                .cloned()
                .collect();
            let fast_files = discover_files(&fast_sources, &ignore_rules, filter);
            // Signal that Claude files need to be discovered later.
            let has_claude_source = sources.iter().any(|s| s.kind == SourceKind::Claude);
            let deferred = if has_claude_source {
                Some(Vec::new()) // empty vec = needs discovery
            } else {
                None
            };
            (fast_files, deferred)
        } else {
            (discover_files(&sources, &ignore_rules, filter), None)
        };

        let now = Instant::now();
        let discovery_interval =
            Duration::from_secs((refresh_every.saturating_mul(3)).clamp(2, 12));

        Ok(Self {
            filter,
            sources,
            ignore_rules,
            pricing,
            worker_count,
            cache_enabled,
            cache_store,
            cache_path,
            cache_dirty: common.rebuild_cache,
            files_cache,
            deferred_claude_files,
            last_discovery_at: now,
            discovery_interval,
            last_sources_refresh_at: now,
            sources_refresh_interval: Duration::from_secs(60),
            last_cache_flush_at: now,
        })
    }

    pub(super) async fn maybe_refresh_sources(&mut self, common: &CommonArgs) -> Result<()> {
        if self.last_sources_refresh_at.elapsed() < self.sources_refresh_interval {
            return Ok(());
        }
        self.last_sources_refresh_at = Instant::now();
        let refreshed = build_sources(common).await?;
        if refreshed.is_empty() || refreshed == self.sources {
            return Ok(());
        }

        self.sources = refreshed;
        self.files_cache.clear();
        self.last_discovery_at = Instant::now() - self.discovery_interval;
        Ok(())
    }

    pub(super) fn maybe_refresh_discovery(&mut self) {
        if !self.files_cache.is_empty()
            && self.last_discovery_at.elapsed() < self.discovery_interval
        {
            return;
        }
        self.files_cache = discover_files(&self.sources, &self.ignore_rules, self.filter);
        self.last_discovery_at = Instant::now();
    }

    /// Returns true if deferred Claude files are still pending.
    pub(super) fn has_deferred_claude(&self) -> bool {
        self.deferred_claude_files.is_some()
    }

    /// Merge deferred Claude files (discovery + parse).  Call this explicitly
    /// after the first fast frame has been rendered so the user sees Codex
    /// data immediately.
    pub(super) fn merge_deferred_claude(&mut self) {
        if let Some(_deferred) = self.deferred_claude_files.take() {
            let claude_sources: Vec<_> = self
                .sources
                .iter()
                .filter(|s| s.kind == SourceKind::Claude)
                .cloned()
                .collect();
            if !claude_sources.is_empty() {
                let mut claude_files =
                    discover_files(&claude_sources, &self.ignore_rules, self.filter);
                self.files_cache.append(&mut claude_files);
                self.files_cache
                    .sort_unstable_by(|a, b| a.path.cmp(&b.path));
                self.files_cache.dedup_by(|a, b| a.path == b.path);
            }
        }
    }

    pub(super) fn load(&mut self, timezone: &TimeZoneMode) -> LoadedUsage {
        self.maybe_refresh_discovery();
        let parsed = parse_files_with_cache(
            &self.files_cache,
            &mut self.cache_store,
            ParseFilesConfig {
                filter: self.filter,
                timezone,
                pricing: self.pricing.clone(),
                worker_count: self.worker_count,
                cache_enabled: self.cache_enabled,
                cache_path: self.cache_path.as_deref(),
                sort_events: false,
            },
        );
        self.cache_dirty |= parsed.cache_dirty;
        self.flush_cache(false);
        parsed.loaded
    }

    pub(super) fn flush_cache(&mut self, force: bool) {
        if !self.cache_enabled || !self.cache_dirty {
            return;
        }
        if !force && self.last_cache_flush_at.elapsed() < Duration::from_secs(10) {
            return;
        }
        if let Some(path) = self.cache_path.as_ref() {
            save_incremental_cache(path, &mut self.cache_store);
            self.cache_dirty = false;
            self.last_cache_flush_at = Instant::now();
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LoadUsageOptions {
    pub sort_events: bool,
}

impl Default for LoadUsageOptions {
    fn default() -> Self {
        Self { sort_events: true }
    }
}

pub(super) async fn load_usage(
    common: &CommonArgs,
    timezone: &TimeZoneMode,
) -> Result<LoadedUsage> {
    load_usage_with_options(common, timezone, LoadUsageOptions::default()).await
}

pub(super) async fn load_usage_unsorted(
    common: &CommonArgs,
    timezone: &TimeZoneMode,
) -> Result<LoadedUsage> {
    load_usage_with_options(common, timezone, LoadUsageOptions { sort_events: false }).await
}

pub(super) async fn load_usage_with_options(
    common: &CommonArgs,
    timezone: &TimeZoneMode,
    options: LoadUsageOptions,
) -> Result<LoadedUsage> {
    let filter = parse_common_filter(common)?;
    let sources = build_sources(common).await?;
    if sources.is_empty() {
        bail!(
            "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
        );
    }

    let ignore_rules = PathIgnoreRules::from_common(common);
    let worker_count = worker_count_from_common(common);
    let pricing_key = cache_schema_key();
    let cache_enabled = !common.no_incremental_cache;
    let cache_path = incremental_cache_path();
    let rebuild_cache = common.rebuild_cache;

    let pricing_fut = load_pricing(common.pricing_file.as_deref(), common.offline);
    let discovery_and_meta_fut = tokio::task::spawn_blocking(move || {
        let (files, cache_store) = rayon::join(
            || discover_files(&sources, &ignore_rules, filter),
            || {
                if cache_enabled && !rebuild_cache {
                    match cache_path.as_ref() {
                        Some(path) => load_incremental_cache_meta(path, &pricing_key),
                        None => {
                            let mut s = IncrementalCacheStore::new(pricing_key.clone());
                            s.meta_only = true;
                            s
                        }
                    }
                } else {
                    IncrementalCacheStore::new(pricing_key.clone())
                }
            },
        );
        (files, cache_store, cache_path)
    });

    let (pricing_res, blocking_res) = tokio::join!(pricing_fut, discovery_and_meta_fut);
    let pricing = Arc::new(pricing_res?);
    let (files, mut cache_store, cache_path) = blocking_res?;
    finish_load_usage(
        &files,
        &mut cache_store,
        filter,
        timezone,
        pricing,
        worker_count,
        cache_enabled,
        cache_path.as_deref(),
        rebuild_cache,
        options.sort_events,
    )
}

pub(super) async fn load_usage_from_discovered(
    common: &CommonArgs,
    timezone: &TimeZoneMode,
    files: &[DiscoveredFile],
    sort_events: bool,
) -> Result<LoadedUsage> {
    let filter = parse_common_filter(common)?;
    let worker_count = worker_count_from_common(common);
    let pricing_key = cache_schema_key();
    let cache_enabled = !common.no_incremental_cache;
    let cache_path = incremental_cache_path();
    let rebuild_cache = common.rebuild_cache;

    let pricing_fut = load_pricing(common.pricing_file.as_deref(), common.offline);
    let cache_path_clone = cache_path.clone();
    let meta_fut = tokio::task::spawn_blocking(move || {
        if cache_enabled && !rebuild_cache {
            match cache_path_clone.as_ref() {
                Some(path) => load_incremental_cache_meta(path, &pricing_key),
                None => IncrementalCacheStore::new(pricing_key),
            }
        } else {
            IncrementalCacheStore::new(pricing_key)
        }
    });

    let (pricing_res, cache_res) = tokio::join!(pricing_fut, meta_fut);
    let pricing = Arc::new(pricing_res?);
    let mut cache_store = cache_res?;
    finish_load_usage(
        files,
        &mut cache_store,
        filter,
        timezone,
        pricing,
        worker_count,
        cache_enabled,
        cache_path.as_deref(),
        rebuild_cache,
        sort_events,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_load_usage(
    files: &[DiscoveredFile],
    cache_store: &mut IncrementalCacheStore,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: Arc<PricingTable>,
    worker_count: usize,
    cache_enabled: bool,
    cache_path: Option<&Path>,
    rebuild_cache: bool,
    sort_events: bool,
) -> Result<LoadedUsage> {
    let parsed = parse_files_with_cache(
        files,
        cache_store,
        ParseFilesConfig {
            filter,
            timezone,
            pricing,
            worker_count,
            cache_enabled,
            cache_path,
            sort_events,
        },
    );
    if cache_enabled
        && (parsed.cache_dirty || rebuild_cache)
        && let Some(path) = cache_path
    {
        save_incremental_cache(path, cache_store);
    }

    Ok(parsed.loaded)
}

pub(super) async fn build_sources(common: &CommonArgs) -> Result<Vec<SourceConfig>> {
    let home = dirs::home_dir().context("Failed to resolve home directory")?;
    let selected = common.selected_sources();
    let provider_selected = |kind: SourceKind| selected.is_empty() || selected.contains(&kind);
    let mut sources = Vec::new();
    for spec in provider_registry() {
        if !(spec.enabled)(common) || !provider_selected(spec.kind) {
            continue;
        }
        let roots = (spec.roots)(common, &home);
        let existing = filter_existing_dirs(roots).await;
        if existing.is_empty() {
            continue;
        }
        sources.push(SourceConfig {
            kind: spec.kind,
            roots: existing,
        });
    }

    Ok(sources)
}

struct ProviderSpec {
    kind: SourceKind,
    accepted_exts: &'static [&'static str],
    enabled: fn(&CommonArgs) -> bool,
    roots: fn(&CommonArgs, &Path) -> Vec<PathBuf>,
}

fn provider_registry() -> [ProviderSpec; 7] {
    [
        ProviderSpec {
            kind: SourceKind::Claude,
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_claude,
            roots: claude_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::Codex,
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_codex,
            roots: codex_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::Gemini,
            accepted_exts: &["jsonl", "db"],
            enabled: |common| !common.no_gemini,
            roots: gemini_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::OpenCode,
            accepted_exts: &["json", "db"],
            enabled: |common| !common.no_opencode,
            roots: opencode_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::Grok,
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_grok,
            roots: grok_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::Pi,
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_pi,
            roots: pi_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::Muse,
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_muse,
            roots: muse_source_roots,
        },
    ]
}

fn claude_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if common.claude_projects_dir.is_empty() {
        return vec![
            home.join(".config").join("claude").join("projects"),
            home.join(".claude").join("projects"),
        ];
    }
    common
        .claude_projects_dir
        .iter()
        .map(|p| expand_user_path(p))
        .collect()
}

fn codex_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if common.codex_sessions_dir.is_empty() {
        let codex_home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        return vec![
            codex_home.join("sessions"),
            codex_home.join("archived_sessions"),
            home.join(".config").join("codex").join("sessions"),
            home.join(".config").join("codex").join("archived_sessions"),
        ];
    }

    let mut out = Vec::new();
    for raw in &common.codex_sessions_dir {
        let path = expand_user_path(raw);
        out.push(path.clone());
        if path.file_name().and_then(|s| s.to_str()) == Some("sessions")
            && let Some(parent) = path.parent()
        {
            out.push(parent.join("archived_sessions"));
        }
    }
    out
}

fn gemini_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if common.gemini_data_dir.is_empty() {
        let base = home.join(".gemini");
        return vec![
            base.join("tmp"),
            base.join("antigravity-cli").join("conversations"),
            base.join("antigravity-cli").join("brain"),
            base.join("antigravity-cli"),
            base.join("antigravity").join("conversations"),
            base.join("antigravity").join("brain"),
            base.join("antigravity"),
            base.join("antigravity-ide").join("conversations"),
            base.join("antigravity-ide").join("brain"),
            base.join("antigravity-ide"),
            base,
        ];
    }
    common
        .gemini_data_dir
        .iter()
        .map(|p| expand_user_path(p))
        .collect()
}

fn grok_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if !common.grok_log_dir.is_empty() {
        return common
            .grok_log_dir
            .iter()
            .map(|p| expand_user_path(p))
            .collect();
    }
    let sessions_dir = home.join(".grok").join("sessions");
    if sessions_dir.is_dir() {
        vec![sessions_dir]
    } else {
        vec![home.join(".grok").join("logs")]
    }
}

fn pi_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if !common.pi_sessions_dir.is_empty() {
        return common
            .pi_sessions_dir
            .iter()
            .map(|p| expand_user_path(p))
            .collect();
    }
    if let Some(session_dir) = std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        let trimmed = session_dir.to_string_lossy();
        let trimmed = trimmed.trim();
        if !trimmed.is_empty() {
            return vec![expand_user_path(trimmed)];
        }
    }
    if let Some(agent_dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let trimmed = agent_dir.to_string_lossy();
        let trimmed = trimmed.trim();
        if !trimmed.is_empty() {
            return vec![expand_user_path(trimmed).join("sessions")];
        }
    }
    vec![home.join(".pi").join("agent").join("sessions")]
}

fn muse_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if !common.muse_sessions_dir.is_empty() {
        return common
            .muse_sessions_dir
            .iter()
            .map(|p| expand_user_path(p))
            .collect();
    }
    let mut candidates = Vec::new();
    if let Some(muse_home) = std::env::var_os("MUSE_HOME") {
        let trimmed = muse_home.to_string_lossy();
        let trimmed = trimmed.trim();
        if !trimmed.is_empty() {
            candidates.push(expand_user_path(trimmed).join("sessions"));
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        let trimmed = xdg.to_string_lossy();
        let trimmed = trimmed.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed).join("muse").join("sessions"));
        }
    }
    candidates.push(
        home.join(".local")
            .join("share")
            .join("muse")
            .join("sessions"),
    );
    #[cfg(target_os = "macos")]
    candidates.push(
        home.join("Library")
            .join("Application Support")
            .join("muse")
            .join("sessions"),
    );
    candidates
}

fn opencode_source_roots(common: &CommonArgs, home: &Path) -> Vec<PathBuf> {
    if !common.opencode_data_dir.is_empty() {
        return common
            .opencode_data_dir
            .iter()
            .map(|p| expand_user_path(p))
            .collect();
    }

    let mut candidates = Vec::<PathBuf>::new();

    if let Some(value) = std::env::var_os("OPENCODE_DATA_DIR") {
        candidates.push(PathBuf::from(value));
    }
    if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(value).join("opencode"));
    }
    candidates.push(home.join(".local").join("share").join("opencode"));

    #[cfg(target_os = "windows")]
    {
        if let Some(value) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(value.clone()).join("opencode"));
            candidates.push(PathBuf::from(value).join("OpenCode"));
        }
        if let Some(value) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(value.clone()).join("opencode"));
            candidates.push(PathBuf::from(value).join("OpenCode"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        candidates.push(
            home.join("Library")
                .join("Application Support")
                .join("opencode"),
        );
        candidates.push(
            home.join("Library")
                .join("Application Support")
                .join("OpenCode"),
        );
    }

    candidates
}

pub(super) async fn filter_existing_dirs(input: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for path in input {
        let normalized = normalize_path(&path);
        if !seen.insert(normalized.clone()) {
            continue;
        }

        if let Ok(meta) = fs::metadata(&normalized).await
            && meta.is_dir()
        {
            out.push(normalized);
        }
    }

    out
}

fn deduplicate_source_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut sorted = roots.to_vec();
    sorted.sort_by_key(|p| p.components().count());
    let mut kept: Vec<PathBuf> = Vec::with_capacity(sorted.len());
    for root in sorted {
        if !kept.iter().any(|ancestor| root.starts_with(ancestor)) {
            kept.push(root);
        }
    }
    kept
}

fn should_prune_subdir(kind: SourceKind, name: &str) -> bool {
    match kind {
        SourceKind::Claude => matches!(name, "tool-results" | "memory"),
        SourceKind::Gemini => matches!(
            name,
            ".system_generated"
                | "skills"
                | "browser_recordings"
                | "scratch"
                | "node_modules"
                | ".git"
        ),
        SourceKind::Grok => matches!(
            name,
            "tool-results"
                | "blobs"
                | "artifacts"
                | "attachments"
                | "workspace"
                | "node_modules"
                | ".git"
        ),
        SourceKind::Codex | SourceKind::OpenCode | SourceKind::Pi | SourceKind::Muse => {
            matches!(name, "node_modules" | ".git")
        }
    }
}

pub(super) fn discover_files(
    sources: &[SourceConfig],
    ignore_rules: &PathIgnoreRules,
    filter: DateFilter,
) -> Vec<DiscoveredFile> {
    let walk_jobs: Vec<(SourceKind, PathBuf)> = sources
        .iter()
        .flat_map(|source| {
            deduplicate_source_roots(&source.roots)
                .into_iter()
                .map(move |root| (source.kind, root))
        })
        .collect();
    let walk_threads = if walk_jobs.len() <= 1 { 4 } else { 2 };

    let mut files: Vec<DiscoveredFile> = walk_jobs
        .par_iter()
        .flat_map_iter(|(kind, root)| {
            discover_files_in_root_with_threads(*kind, root, ignore_rules, filter, walk_threads)
        })
        .collect();

    files.par_sort_unstable_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    files
}

#[allow(dead_code)]
pub(super) fn discover_files_in_root(
    kind: SourceKind,
    root: &Path,
    ignore_rules: &PathIgnoreRules,
    filter: DateFilter,
) -> Vec<DiscoveredFile> {
    let threads = thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(4);
    discover_files_in_root_with_threads(kind, root, ignore_rules, filter, threads)
}

fn discover_files_in_root_with_threads(
    kind: SourceKind,
    root: &Path,
    ignore_rules: &PathIgnoreRules,
    _filter: DateFilter,
    threads: usize,
) -> Vec<DiscoveredFile> {
    // NOTE:
    // Do not short-circuit Codex discovery by directory date partition.
    // A Codex session file can continue receiving events on later days while
    // staying under its original directory (session start date), so partition
    // pruning can miss valid events and undercount filtered ranges.
    let mut initial = Vec::new();

    if kind == SourceKind::OpenCode {
        // Always merge both SQLite and legacy message logs when present.
        let db_path = root.join("opencode.db");
        if db_path.is_file() && !ignore_rules.should_skip_path(&db_path) {
            initial.push(DiscoveredFile {
                source: kind,
                root: root.to_path_buf(),
                path: normalized_discovered_path(&db_path),
            });
        }
    }
    let rules = ignore_rules.clone();
    let mut builder = WalkBuilder::new(root);
    builder
        .threads(threads.max(1))
        .follow_links(false)
        // keep hidden entries visible because source roots often start with '.'
        .hidden(false)
        // source roots may live inside git repos with .gitignore rules that
        // exclude log files (e.g. ~/.claude has `projects/*/` ignored); always
        // walk them regardless of ignore files.
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_type().is_some_and(|ft| ft.is_dir())
                && let Some(name) = entry.file_name().to_str()
                && should_prune_subdir(kind, name)
            {
                return false;
            }
            !rules.should_skip_dir(entry.path())
        });
    if kind == SourceKind::Grok {
        builder.max_depth(Some(4));
    }

    struct LocalCollector<'a> {
        out: &'a Mutex<Vec<DiscoveredFile>>,
        buf: Vec<DiscoveredFile>,
    }

    impl Drop for LocalCollector<'_> {
        fn drop(&mut self) {
            if !self.buf.is_empty()
                && let Ok(mut guard) = self.out.lock()
            {
                guard.append(&mut self.buf);
            }
        }
    }

    let out = Mutex::new(initial);
    let root_buf = root.to_path_buf();
    builder.build_parallel().run(|| {
        let out = &out;
        let root_buf = &root_buf;
        let mut collector = LocalCollector {
            out,
            buf: Vec::new(),
        };
        Box::new(move |entry_res| {
            let Ok(entry) = entry_res else {
                return ignore::WalkState::Continue;
            };
            let path = entry.path();
            let is_file = entry
                .file_type()
                .map(|file_type| file_type.is_file())
                .unwrap_or(false);
            if !is_file || ignore_rules.should_skip_path(path) {
                return ignore::WalkState::Continue;
            }

            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                return ignore::WalkState::Continue;
            };
            if !provider_accepts_extension(kind, ext) {
                return ignore::WalkState::Continue;
            }

            if kind == SourceKind::OpenCode {
                // Restrict OpenCode discovery to message JSONs; skip db/log/snapshots.
                let normalized = path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                if !normalized.contains("/storage/message/") {
                    return ignore::WalkState::Continue;
                }
            }

            if kind == SourceKind::Grok {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if file_name != "updates.jsonl" && file_name != "unified.jsonl" {
                    return ignore::WalkState::Continue;
                }
            }

            if kind == SourceKind::Muse {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if file_name != "session.jsonl" {
                    return ignore::WalkState::Continue;
                }
            }

            collector.buf.push(DiscoveredFile {
                source: kind,
                root: root_buf.clone(),
                path: normalized_discovered_path(path),
            });
            if collector.buf.len() >= 256
                && let Ok(mut guard) = collector.out.lock()
            {
                guard.append(&mut collector.buf);
            }
            ignore::WalkState::Continue
        })
    });

    out.into_inner().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn dedupe_opencode_events(events: &mut Vec<UsageEvent>) {
    fn opencode_msg_id(file_path: &str) -> Option<String> {
        if let Some((_, suffix)) = file_path.rsplit_once('#')
            && !suffix.is_empty()
        {
            return Some(suffix.to_string());
        }

        let file_name = Path::new(file_path).file_name()?.to_str()?;
        let stem = file_name.strip_suffix(".json")?;
        if !stem.is_empty() {
            return Some(stem.to_string());
        }

        None
    }

    let mut seen_msg = std::collections::HashMap::<String, usize>::new();
    let mut seen_fallback = HashSet::new();

    let mut out = Vec::with_capacity(events.len());
    for mut event in events.drain(..) {
        if event.source != SourceKind::OpenCode {
            out.push(event);
            continue;
        }

        if let Some(msg_id) = opencode_msg_id(&event.file_path) {
            if let Some(&existing_idx) = seen_msg.get(&msg_id) {
                let existing = &mut out[existing_idx];
                if existing.session.is_empty() && !event.session.is_empty() {
                    existing.session = std::mem::take(&mut event.session);
                }
                if existing.project.is_none() && event.project.is_some() {
                    existing.project = event.project.take();
                }
                continue;
            }
            seen_msg.insert(msg_id, out.len());
            out.push(event);
            continue;
        }

        let fallback = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            event.timestamp.timestamp_millis(),
            event.model,
            event.session,
            event.project.as_deref().unwrap_or_default(),
            event.usage.input_tokens,
            event.usage.cache_creation_input_tokens,
            event.usage.cache_read_input_tokens,
            event.usage.output_tokens,
            event.usage.reasoning_output_tokens
        );
        if seen_fallback.insert(fallback) {
            out.push(event);
        }
    }

    *events = out;
}

fn provider_accepts_extension(kind: SourceKind, ext: &str) -> bool {
    provider_registry()
        .iter()
        .find(|spec| spec.kind == kind)
        .is_some_and(|meta| {
            meta.accepted_exts
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

pub(super) struct ParsedRecord {
    event: UsageEvent,
    pricing_ctx: PricingContext,
    dedupe_hash: Option<u64>,
    used_unknown_pricing: bool,
}

pub(super) enum MultiLineResult {
    Parsed(Vec<ParsedRecord>),
    InvalidJson,
    MissingUsage,
}

fn merge_usage_max(dst: &mut UsageAccumulator, src: UsageAccumulator) -> bool {
    let mut changed = false;
    if src.input_tokens > dst.input_tokens {
        dst.input_tokens = src.input_tokens;
        changed = true;
    }
    if src.cache_creation_input_tokens > dst.cache_creation_input_tokens {
        dst.cache_creation_input_tokens = src.cache_creation_input_tokens;
        changed = true;
    }
    if src.cache_read_input_tokens > dst.cache_read_input_tokens {
        dst.cache_read_input_tokens = src.cache_read_input_tokens;
        changed = true;
    }
    if src.output_tokens > dst.output_tokens {
        dst.output_tokens = src.output_tokens;
        changed = true;
    }
    if src.reasoning_output_tokens > dst.reasoning_output_tokens {
        dst.reasoning_output_tokens = src.reasoning_output_tokens;
        changed = true;
    }
    changed
}

fn merge_ctx_max(dst: &mut PricingContext, src: PricingContext) -> bool {
    let mut changed = false;
    if src.cache_creation_1h_tokens > dst.cache_creation_1h_tokens {
        dst.cache_creation_1h_tokens = src.cache_creation_1h_tokens;
        changed = true;
    }
    match (dst.reported_cost_usd, src.reported_cost_usd) {
        (None, Some(s)) => {
            dst.reported_cost_usd = Some(s);
            changed = true;
        }
        (Some(d), Some(s)) if s > d => {
            dst.reported_cost_usd = Some(s);
            changed = true;
        }
        _ => {}
    }
    changed
}

fn dedupe_claude_events_max_merge(events: &mut Vec<InternalEvent>, pricing: &PricingTable) {
    let mut seen = HashMap::<u64, usize>::new();
    let mut out = Vec::<InternalEvent>::with_capacity(events.len());
    for item in events.drain(..) {
        if item.event.source != SourceKind::Claude {
            out.push(item);
            continue;
        }
        let Some(hash) = item.dedupe_hash else {
            out.push(item);
            continue;
        };
        if let Some(&idx) = seen.get(&hash) {
            let held = &mut out[idx];
            let mut changed = merge_usage_max(&mut held.event.usage, item.event.usage);
            changed |= merge_ctx_max(&mut held.pricing_ctx, item.pricing_ctx);
            if changed {
                held.event.usage.cost_usd = pricing
                    .estimate_cost_with_context(
                        &held.event.model,
                        held.event.usage,
                        held.pricing_ctx,
                    )
                    .unwrap_or(0.0);
            }
        } else {
            seen.insert(hash, out.len());
            out.push(item);
        }
    }
    *events = out;
}

fn dedupe_hash_events(events: &mut Vec<InternalEvent>) {
    let mut seen = HashSet::<(SourceKind, u64)>::new();
    let mut out = Vec::<InternalEvent>::with_capacity(events.len());
    for item in events.drain(..) {
        if matches!(
            item.event.source,
            SourceKind::Grok | SourceKind::Pi | SourceKind::Muse
        ) && let Some(hash) = item.dedupe_hash
            && !seen.insert((item.event.source, hash))
        {
            continue;
        }
        out.push(item);
    }
    *events = out;
}

pub(super) fn parse_files_concurrently(
    files: Vec<FileParseJob>,
    workers: usize,
    filter: DateFilter,
    timezone: TimeZoneMode,
    pricing: Arc<PricingTable>,
    stats: Arc<ParseStatsAtomic>,
    cache_enabled: bool,
) -> WorkerParseOutput {
    if files.is_empty() {
        return WorkerParseOutput::default();
    }

    let (tx, rx) = bounded::<FileParseJob>(4096);

    let producer = {
        let tx = tx.clone();
        thread::spawn(move || {
            for file in files {
                if tx.send(file).is_err() {
                    break;
                }
            }
        })
    };
    drop(tx);

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let rx = rx.clone();
        let pricing = pricing.clone();
        let stats = stats.clone();
        let timezone = timezone.clone();

        let handle = thread::spawn(move || {
            worker_loop(rx, filter, &timezone, &pricing, &stats, cache_enabled)
        });
        handles.push(handle);
    }

    let _ = producer.join();
    let mut out = WorkerParseOutput::default();
    for handle in handles {
        if let Ok(mut worker) = handle.join() {
            out.events.append(&mut worker.events);
            out.cache_updates.append(&mut worker.cache_updates);
        }
    }
    out
}

pub(super) fn worker_loop(
    rx: Receiver<FileParseJob>,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    cache_enabled: bool,
) -> WorkerParseOutput {
    let mut output = WorkerParseOutput::default();
    while let Ok(job) = rx.recv() {
        let cache_key = cache_enabled.then(|| job.cache_key.clone());
        if let Some(parsed) =
            parse_single_file(job, filter, timezone, pricing, stats, cache_enabled)
        {
            output.events.extend(parsed.events);
            if let Some(cache_key) = cache_key {
                output.cache_updates.push((cache_key, parsed.cache_entry));
            }
        }
    }
    output
}

pub(super) fn parse_single_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    cache_enabled: bool,
) -> Option<ParsedFileOutput> {
    if job.file.source == SourceKind::Gemini {
        let ext = job
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext.eq_ignore_ascii_case("db") {
            return parse_antigravity_db_file(job, filter, timezone, pricing, stats, cache_enabled);
        }
    }

    if job.file.source == SourceKind::OpenCode {
        let ext = job
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext.eq_ignore_ascii_case("db") {
            return parse_opencode_db_file(job, filter, timezone, pricing, stats, cache_enabled);
        }
        return parse_opencode_message_file(job, filter, timezone, pricing, stats, cache_enabled);
    }

    let input = match File::open(&job.file.path) {
        Ok(f) => f,
        Err(_) => {
            stats.files_open_failed.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };

    let (session, project) = derive_session_meta(&job.file);
    let file_path = job.file.path.display().to_string();

    let mut reader = BufReader::with_capacity(64 * 1024, input);
    let mut codex_state = CodexParseState::default();
    let mut pi_session_id = String::new();

    let mut local_events: Vec<InternalEvent> = Vec::new();
    let mut cached_events: Vec<CachedUsageEvent> = Vec::new();
    let mut claude_cached_idx: HashMap<u64, usize> = HashMap::new();
    let mut claude_local_idx: HashMap<u64, usize> = HashMap::new();
    let mut lines_total = 0usize;
    let mut lines_parsed = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;
    let mut base_stats = CachedFileStats::default();
    let mut parsed_offset = 0u64;

    if let ParseStrategy::Incremental { ref base_cache } = job.strategy {
        let fallback_offset = base_cache.fingerprint.size;
        let seek_offset = base_cache
            .parsed_offset
            .max(fallback_offset)
            .min(job.fingerprint.size);
        if seek_offset > 0 && reader.seek(SeekFrom::Start(seek_offset)).is_ok() {
            parsed_offset = seek_offset;
            base_stats = base_cache.stats.clone();
            cached_events.extend(base_cache.events.iter().cloned());
            for (idx, ev) in cached_events.iter().enumerate() {
                if let Some(hash) = ev.dedupe_hash {
                    claude_cached_idx.insert(hash, idx);
                }
            }
            local_events.extend(hydrate_cached_internal_events(
                &job.file, base_cache, filter, timezone, pricing, stats,
            ));
            for (idx, item) in local_events.iter().enumerate() {
                if let Some(hash) = item.dedupe_hash {
                    claude_local_idx.insert(hash, idx);
                }
            }
            if job.file.source == SourceKind::Codex {
                codex_state.current_model = base_cache.codex_last_model.clone();
                codex_state.previous_totals = base_cache.codex_last_totals;
                codex_state.service_tier = base_cache.codex_service_tier;
                codex_state.fork_anchor_ts = base_cache
                    .codex_fork_anchor_ms
                    .and_then(DateTime::from_timestamp_millis);
                codex_state.request_ledger_active = base_cache.codex_request_ledger_active;
            } else if job.file.source == SourceKind::Pi
                && let Some(prev_session) = base_cache
                    .events
                    .iter()
                    .rev()
                    .find_map(|ev| ev.session.clone())
                    .filter(|s| !s.is_empty())
            {
                pi_session_id = prev_session;
            }
        } else {
            let _ = reader.seek(SeekFrom::Start(0));
        }
    }
    let hydrated_len = local_events.len();
    let mut day_cache = LocalDayCache::new(timezone);

    let mut line = String::with_capacity(512);
    loop {
        line.clear();
        let bytes = match reader.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes == 0 {
            break;
        }
        if bytes > MAX_JSON_LINE_BYTES {
            lines_invalid_json += 1;
            continue;
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }

        lines_total += 1;

        if should_skip_parse_by_line_prefix(job.file.source, &line) {
            lines_missing_usage += 1;
            continue;
        }

        let multi = match job.file.source {
            SourceKind::Claude => parse_claude_usage_line_records(&line),
            SourceKind::Codex => parse_codex_usage_line_records(&line, &mut codex_state, pricing),
            SourceKind::Gemini => match parse_gemini_usage_line(&line, pricing) {
                ParseLineResult::Parsed(p) => MultiLineResult::Parsed(vec![ParsedRecord {
                    event: p.event,
                    pricing_ctx: PricingContext::default(),
                    dedupe_hash: None,
                    used_unknown_pricing: p.used_unknown_pricing,
                }]),
                ParseLineResult::InvalidJson => MultiLineResult::InvalidJson,
                ParseLineResult::MissingUsage => MultiLineResult::MissingUsage,
            },
            SourceKind::OpenCode => unreachable!("OpenCode is parsed as whole-file JSON"),
            SourceKind::Grok => parse_grok_usage_line_records(&line, pricing),
            SourceKind::Pi => parse_pi_usage_line_records(&line, &mut pi_session_id, pricing),
            SourceKind::Muse => parse_muse_usage_line_records(&line, pricing),
        };

        let records = match multi {
            MultiLineResult::Parsed(records) => records,
            MultiLineResult::InvalidJson => {
                lines_invalid_json += 1;
                continue;
            }
            MultiLineResult::MissingUsage => {
                lines_missing_usage += 1;
                continue;
            }
        };

        for mut parsed in records {
            if job.file.source == SourceKind::Claude
                && let Some(hash) = parsed.dedupe_hash
            {
                let mut is_dup = false;
                if let Some(&c_idx) = claude_cached_idx.get(&hash) {
                    is_dup = true;
                    let c_ev = &mut cached_events[c_idx];
                    merge_usage_max(&mut c_ev.usage, parsed.event.usage);
                    if parsed.pricing_ctx.cache_creation_1h_tokens > c_ev.cache_creation_1h_tokens {
                        c_ev.cache_creation_1h_tokens = parsed.pricing_ctx.cache_creation_1h_tokens;
                    }
                    if let Some(rc) = parsed.pricing_ctx.reported_cost_usd {
                        c_ev.reported_cost_usd =
                            Some(c_ev.reported_cost_usd.map_or(rc, |prev| prev.max(rc)));
                    }
                }
                if let Some(&l_idx) = claude_local_idx.get(&hash) {
                    is_dup = true;
                    let l_ev = &mut local_events[l_idx];
                    merge_usage_max(&mut l_ev.event.usage, parsed.event.usage);
                    merge_ctx_max(&mut l_ev.pricing_ctx, parsed.pricing_ctx);
                }
                if is_dup {
                    continue;
                }
            }

            if parsed.used_unknown_pricing {
                lines_unknown_pricing += 1;
            }

            if parsed.event.session.is_empty() {
                parsed.event.session.clone_from(&session);
            }
            if parsed.event.project.is_none() {
                parsed.event.project = project.clone();
            }
            parsed.event.file_path.clone_from(&file_path);

            if cache_enabled {
                let c_idx = cached_events.len();
                cached_events.push(cached_usage_event_for_file(
                    &parsed.event,
                    parsed.pricing_ctx,
                    parsed.dedupe_hash,
                    &file_path,
                ));
                if job.file.source == SourceKind::Claude
                    && let Some(hash) = parsed.dedupe_hash
                {
                    claude_cached_idx.insert(hash, c_idx);
                }
            }

            if !date_filter_allows_cached(filter, parsed.event.timestamp, &mut day_cache) {
                lines_filtered += 1;
                continue;
            }

            let l_idx = local_events.len();
            local_events.push(InternalEvent {
                event: parsed.event,
                pricing_ctx: parsed.pricing_ctx,
                dedupe_hash: parsed.dedupe_hash,
            });
            if job.file.source == SourceKind::Claude
                && let Some(hash) = parsed.dedupe_hash
            {
                claude_local_idx.insert(hash, l_idx);
            }
            lines_parsed += 1;
        }
    }

    if job.file.source == SourceKind::Claude {
        for (idx, item) in local_events.iter_mut().enumerate() {
            match pricing.estimate_cost_with_context(
                &item.event.model,
                item.event.usage,
                item.pricing_ctx,
            ) {
                Some(cost) => item.event.usage.cost_usd = cost,
                None => {
                    item.event.usage.cost_usd = 0.0;
                    if idx >= hydrated_len {
                        lines_unknown_pricing += 1;
                    }
                }
            }
        }
    }

    stats.lines_total.fetch_add(lines_total, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(lines_missing_usage, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(lines_unknown_pricing, Ordering::Relaxed);
    stats
        .lines_filtered
        .fetch_add(lines_filtered, Ordering::Relaxed);
    stats
        .lines_parsed
        .fetch_add(lines_parsed, Ordering::Relaxed);

    let cache_entry = CachedFileEntry {
        fingerprint: job.fingerprint,
        stats: CachedFileStats {
            lines_total: base_stats.lines_total + lines_total,
            lines_invalid_json: base_stats.lines_invalid_json + lines_invalid_json,
            lines_missing_usage: base_stats.lines_missing_usage + lines_missing_usage,
            lines_unknown_pricing: base_stats.lines_unknown_pricing + lines_unknown_pricing,
        },
        events: cached_events,
        parsed_offset: job.fingerprint.size.max(parsed_offset),
        codex_last_model: codex_state.current_model,
        codex_last_totals: codex_state.previous_totals,
        codex_service_tier: codex_state.service_tier,
        codex_fork_anchor_ms: codex_state.fork_anchor_ts.map(|ts| ts.timestamp_millis()),
        codex_request_ledger_active: codex_state.request_ledger_active,
    };

    Some(ParsedFileOutput {
        events: local_events,
        cache_entry,
    })
}

pub(super) fn should_skip_parse_by_line_prefix(source: SourceKind, line: &str) -> bool {
    let markers = provider_fast_line_markers(source);
    if markers.is_empty() {
        return false;
    }
    !markers.iter().any(|marker| line.contains(marker))
}

fn provider_fast_line_markers(source: SourceKind) -> &'static [&'static str] {
    match source {
        SourceKind::Codex => &[
            "\"token_count\"",
            "\"token_usage_record\"",
            "\"turn_context\"",
            "\"thread_settings_applied\"",
            "\"session_meta\"",
        ],
        SourceKind::Claude => &["\"usage\""],
        SourceKind::Gemini => &["\"tokens\"", "\"usage\""],
        SourceKind::OpenCode => &[],
        SourceKind::Grok => &["\"turn_completed\"", "\"shell.turn.inference_done\""],
        SourceKind::Pi => &["\"usage\"", "\"session\""],
        SourceKind::Muse => &["\"model_completed\"", "\"automated_review_completed\""],
    }
}

pub(super) fn derive_session_meta(file: &DiscoveredFile) -> (String, Option<String>) {
    let relative = file
        .path
        .strip_prefix(&file.root)
        .unwrap_or(file.path.as_path())
        .to_path_buf();

    let session = match file.source {
        SourceKind::OpenCode => {
            // `.../storage/message/<sessionID>/<msg>.json`
            let normalized = relative.to_string_lossy().replace('\\', "/");
            normalized
                .split("/storage/message/")
                .nth(1)
                .and_then(|tail| tail.split('/').next())
                .unwrap_or_default()
                .to_string()
        }
        SourceKind::Grok | SourceKind::Muse => {
            // `<...>/<sessionID>/(updates.jsonl|session.jsonl)` -> `<sessionID>`
            let normalized = relative.to_string_lossy().replace('\\', "/");
            let parts: Vec<&str> = normalized.split('/').collect();
            if parts.len() >= 2 {
                parts[parts.len() - 2].to_string()
            } else {
                relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/")
            }
        }
        _ => relative
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/"),
    };

    let raw_project = relative
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .filter(|s| !s.is_empty() && s != ".");

    let project = match file.source {
        SourceKind::Claude | SourceKind::Pi => raw_project.map(|p| decode_claude_project_dir(&p)),
        SourceKind::Codex => extract_codex_project_from_file(&file.path).or(raw_project),
        SourceKind::Gemini | SourceKind::OpenCode | SourceKind::Muse => None,
        SourceKind::Grok => raw_project.map(|p| decode_grok_project_dir(&p)),
    };

    (session, project)
}

/// Decode Claude project directory name like `-Users-hanbu-MyApps-letxn-Codes-buidlme`
/// into the last meaningful path segment(s).
pub(super) fn decode_claude_project_dir(dir_name: &str) -> String {
    let stripped = dir_name.trim_start_matches('-');
    let parts: Vec<&str> = stripped.split('-').collect();

    // Skip common path prefixes
    let skip = ["Users", "home", "root", "var", "tmp", "opt"];
    let meaningful: Vec<&str> = parts
        .iter()
        .copied()
        .skip_while(|p| skip.contains(p) || p.len() <= 2)
        .collect();

    // Take last 2 meaningful segments for a concise project name
    if meaningful.len() >= 2 {
        meaningful[meaningful.len() - 2..].join("/")
    } else if let Some(last) = meaningful.last() {
        last.to_string()
    } else {
        dir_name.to_string()
    }
}

/// Decode Grok URL-encoded cwd folder (e.g. `%2FUsers%2Ffelix%2FProjects%2Fequity-signals`)
/// into a clean project name (`equity-signals`).
pub(super) fn decode_grok_project_dir(dir_name: &str) -> String {
    let unencoded = if dir_name.contains("%2F") || dir_name.contains("%2f") {
        percent_decode_str(dir_name)
    } else {
        dir_name.to_string()
    };
    let trimmed = unencoded.trim_end_matches('/');
    if let Some(name) = trimmed.split('/').next_back() {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    dir_name.to_string()
}

fn percent_decode_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                if let Ok(byte) = u8::from_str_radix(&format!("{}{}", c1, c2), 16) {
                    out.push(byte as char);
                    continue;
                }
                out.push('%');
                out.push(c1);
                out.push(c2);
            } else {
                out.push('%');
                if let Some(c1) = h1 {
                    out.push(c1);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Read the first line of a Codex JSONL file to extract project from session_meta cwd.
pub(super) fn extract_codex_project_from_file(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;

    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }

    let cwd = value.get("payload")?.get("cwd")?.as_str()?;
    // Extract last path segment as project name
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

/// Extract a one-line title from a session JSONL file by reading the first
/// user message. Reads at most 100 lines to stay fast.
pub(super) fn extract_session_title(source: SourceKind, path: &Path) -> String {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let reader = BufReader::new(file);
    let max_lines = 100;

    for (i, line_result) in reader.lines().enumerate() {
        if i >= max_lines {
            break;
        }
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match source {
            SourceKind::Claude => {
                let entry_type = value.get("type").and_then(Value::as_str).unwrap_or("");
                if entry_type == "summary" {
                    if let Some(s) = value.get("summary").and_then(Value::as_str) {
                        let t = s.chars().take(80).collect::<String>();
                        if !t.is_empty() {
                            return t;
                        }
                    }
                }
                if entry_type == "human" || entry_type == "user" {
                    if let Some(content) = extract_message_text(&value) {
                        let trimmed = content.trim();
                        // Skip system-like content (XML tags, slash commands)
                        if trimmed.starts_with('<') || trimmed.starts_with('/') {
                            continue;
                        }
                        return trimmed.chars().take(80).collect();
                    }
                }
            }
            SourceKind::Codex => {
                let msg_type = value.get("type").and_then(Value::as_str).unwrap_or("");
                if msg_type == "response_item" {
                    let payload = value.get("payload");
                    let role = payload.and_then(|p| p.get("role")).and_then(Value::as_str);
                    if role == Some("user") {
                        if let Some(content) = payload.and_then(extract_codex_user_prompt) {
                            return content.chars().take(80).collect();
                        }
                    }
                }
            }
            SourceKind::Gemini => {
                let entry_type = value.get("type").and_then(Value::as_str).unwrap_or("");
                if entry_type == "user" {
                    let content = value.get("content");
                    if let Some(arr) = content.and_then(Value::as_array) {
                        for item in arr {
                            if let Some(text) = item.get("text").and_then(Value::as_str) {
                                let t = text.trim();
                                if !t.is_empty() {
                                    return t.chars().take(80).collect();
                                }
                            }
                        }
                    }
                }
            }
            SourceKind::OpenCode | SourceKind::Pi | SourceKind::Muse => {}
            // Grok has no per-session file to preview from (unified.jsonl is
            // shared across every session/project).
            SourceKind::Grok => {}
        }
    }
    String::new()
}

/// Extract text content from a Claude message entry.
pub(super) fn extract_message_text(value: &Value) -> Option<String> {
    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| value.get("content"))?;

    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        for item in arr {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// Extract the real user prompt from a Codex user message, skipping system
/// context like AGENTS.md, environment_context, etc.
pub(super) fn extract_codex_user_prompt(payload: &Value) -> Option<String> {
    let content = payload.get("content")?;
    if let Some(s) = content.as_str() {
        if !is_codex_system_content(s) {
            return Some(s.to_string());
        }
        return None;
    }
    if let Some(arr) = content.as_array() {
        // The last input_text item is typically the real user prompt;
        // earlier items are AGENTS.md, env context, etc.
        for item in arr.iter().rev() {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_type == "input_text" || item_type == "text" {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    if !is_codex_system_content(t) {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

pub(super) fn is_codex_system_content(s: &str) -> bool {
    let t = s.trim();
    t.starts_with("# AGENTS.md")
        || t.starts_with("<environment_context>")
        || t.starts_with("<context>")
        || t.starts_with("<system>")
        || t.starts_with("AGENTS.md")
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTimestamp {
    Str(String),
    Int(i64),
    Float(f64),
}

impl RawTimestamp {
    fn to_utc(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Str(s) => DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc)),
            Self::Int(raw) => {
                if *raw > 10_000_000_000 {
                    DateTime::from_timestamp_millis(*raw)
                } else {
                    DateTime::from_timestamp(*raw, 0)
                }
            }
            Self::Float(f) => {
                if !f.is_finite() {
                    return None;
                }
                let raw = *f as i64;
                if raw > 10_000_000_000 {
                    DateTime::from_timestamp_millis(raw)
                } else {
                    DateTime::from_timestamp(raw, 0)
                }
            }
        }
    }
}

fn de_opt_u64_clamped<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct U64Visitor;
    impl<'de> serde::de::Visitor<'de> for U64Visitor {
        type Value = Option<u64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("integer, numeric string, or null")
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.max(0) as u64))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
            if v.is_finite() && v >= 0.0 {
                Ok(Some(v as u64))
            } else {
                Ok(Some(0))
            }
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> std::result::Result<Self::Value, E> {
            Ok(s.parse::<u64>().ok())
        }
    }
    deserializer.deserialize_any(U64Visitor)
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeCacheCreationSplit {
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeOutputDetails {
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeIteration {
    #[serde(rename = "type", default)]
    iter_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_read_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning_output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation: Option<ClaudeCacheCreationSplit>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeUsageObj {
    #[serde(default)]
    model: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "inputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_creation_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheCreationInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_creation_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_read_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheReadInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_read_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cached_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cachedInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cached_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    output_tokens: Option<u64>,
    #[serde(
        default,
        rename = "outputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    output_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning_output_tokens: Option<u64>,
    #[serde(
        default,
        rename = "reasoningOutputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    reasoning_output_tokens_camel: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<ClaudeOutputDetails>,
    #[serde(default)]
    cache_creation: Option<ClaudeCacheCreationSplit>,
    #[serde(default)]
    speed: Option<String>,
    #[serde(default)]
    iterations: Option<Vec<ClaudeIteration>>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeMessageMetadata {
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeMessageObj {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    timestamp: Option<RawTimestamp>,
    #[serde(default)]
    created_at: Option<RawTimestamp>,
    #[serde(default)]
    metadata: Option<ClaudeMessageMetadata>,
    #[serde(default)]
    usage: Option<ClaudeUsageObj>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudePayloadFallback {
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeLineRecord {
    #[serde(rename = "type", default)]
    entry_type: Option<String>,
    #[serde(default)]
    timestamp: Option<RawTimestamp>,
    #[serde(default)]
    created_at: Option<RawTimestamp>,
    #[serde(default, rename = "createdAt")]
    created_at_camel: Option<RawTimestamp>,
    #[serde(default, rename = "messageId")]
    message_id: Option<String>,
    #[serde(default, rename = "requestId")]
    request_id: Option<String>,
    #[serde(default, rename = "request_id")]
    request_id_snake: Option<String>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default, rename = "session_id")]
    session_id_snake: Option<String>,
    #[serde(default, rename = "costUSD")]
    cost_usd: Option<f64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    payload: Option<ClaudePayloadFallback>,
    #[serde(default)]
    message: Option<ClaudeMessageObj>,
    #[serde(default)]
    usage: Option<ClaudeUsageObj>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "inputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_creation_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheCreationInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_creation_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_read_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheReadInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_read_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cached_input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cachedInputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cached_input_tokens_camel: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    output_tokens: Option<u64>,
    #[serde(
        default,
        rename = "outputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    output_tokens_camel: Option<u64>,
}

fn parse_claude_usage_line_records(line: &str) -> MultiLineResult {
    let rec: ClaudeLineRecord = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return MultiLineResult::InvalidJson,
    };

    if rec
        .entry_type
        .as_deref()
        .is_some_and(|entry_type| entry_type != "assistant")
    {
        return MultiLineResult::MissingUsage;
    }

    let timestamp = rec
        .timestamp
        .as_ref()
        .and_then(RawTimestamp::to_utc)
        .or_else(|| rec.created_at.as_ref().and_then(RawTimestamp::to_utc))
        .or_else(|| rec.created_at_camel.as_ref().and_then(RawTimestamp::to_utc))
        .or_else(|| {
            rec.message
                .as_ref()
                .and_then(|m| m.timestamp.as_ref().and_then(RawTimestamp::to_utc))
        })
        .or_else(|| {
            rec.message
                .as_ref()
                .and_then(|m| m.created_at.as_ref().and_then(RawTimestamp::to_utc))
        });
    let Some(timestamp) = timestamp else {
        return MultiLineResult::MissingUsage;
    };

    let msg_ref = rec.message.as_ref();
    let usage_ref = msg_ref
        .and_then(|m| m.usage.as_ref())
        .or(rec.usage.as_ref());

    let model = msg_ref
        .and_then(|m| m.model.as_deref())
        .or_else(|| usage_ref.and_then(|u| u.model.as_deref()))
        .or(rec.model.as_deref())
        .or_else(|| rec.payload.as_ref().and_then(|p| p.model.as_deref()))
        .or_else(|| {
            msg_ref
                .and_then(|m| m.metadata.as_ref())
                .and_then(|meta| meta.model.as_deref())
        })
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "<synthetic>");
    let Some(model) = model else {
        return MultiLineResult::MissingUsage;
    };

    let usage = UsageAccumulator {
        input_tokens: usage_ref
            .and_then(|u| u.input_tokens.or(u.input_tokens_camel))
            .or(rec.input_tokens)
            .or(rec.input_tokens_camel)
            .unwrap_or(0),
        cache_creation_input_tokens: usage_ref
            .and_then(|u| {
                u.cache_creation_input_tokens
                    .or(u.cache_creation_input_tokens_camel)
            })
            .or(rec.cache_creation_input_tokens)
            .or(rec.cache_creation_input_tokens_camel)
            .unwrap_or(0),
        cache_read_input_tokens: usage_ref
            .and_then(|u| {
                u.cache_read_input_tokens
                    .or(u.cache_read_input_tokens_camel)
                    .or(u.cached_input_tokens)
                    .or(u.cached_input_tokens_camel)
            })
            .or(rec.cache_read_input_tokens)
            .or(rec.cache_read_input_tokens_camel)
            .or(rec.cached_input_tokens)
            .or(rec.cached_input_tokens_camel)
            .unwrap_or(0),
        output_tokens: usage_ref
            .and_then(|u| u.output_tokens.or(u.output_tokens_camel))
            .or(rec.output_tokens)
            .or(rec.output_tokens_camel)
            .unwrap_or(0),
        reasoning_output_tokens: usage_ref
            .and_then(|u| {
                u.reasoning_output_tokens
                    .or(u.reasoning_output_tokens_camel)
                    .or_else(|| {
                        u.output_tokens_details
                            .as_ref()
                            .and_then(|d| d.reasoning_tokens)
                    })
            })
            .unwrap_or(0),
        cost_usd: 0.0,
    };

    if usage.total_tokens() == 0 {
        return MultiLineResult::MissingUsage;
    }

    let message_id = msg_ref
        .and_then(|m| m.id.as_deref())
        .or(rec.message_id.as_deref());
    let request_id = rec
        .request_id
        .as_deref()
        .or(rec.request_id_snake.as_deref());
    let dedupe_base = if message_id.is_some() || request_id.is_some() {
        Some(format!(
            "{}:{}",
            message_id.unwrap_or(""),
            request_id.unwrap_or("")
        ))
    } else {
        None
    };

    let speed = if usage_ref
        .and_then(|u| u.speed.as_deref())
        .is_some_and(|s| s == "fast")
    {
        ServiceSpeed::Fast
    } else {
        ServiceSpeed::Standard
    };
    let cw_1h = usage_ref
        .and_then(|u| u.cache_creation.as_ref())
        .and_then(|c| c.ephemeral_1h_input_tokens)
        .unwrap_or(0);
    let reported_cost_usd = rec.cost_usd.filter(|c| c.is_finite() && *c >= 0.0);
    let session = rec.session_id.or(rec.session_id_snake).unwrap_or_default();

    let mut out = Vec::with_capacity(1);
    out.push(ParsedRecord {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Claude,
            model: model.to_string(),
            session: session.clone(),
            project: None,
            file_path: String::new(),
            usage,
        },
        pricing_ctx: PricingContext {
            cache_creation_1h_tokens: cw_1h,
            speed,
            model_calls: 1,
            reported_cost_usd,
        },
        dedupe_hash: dedupe_base.as_deref().map(|k| fnv1a_64(k.as_bytes())),
        used_unknown_pricing: false,
    });

    if let Some(u) = usage_ref {
        for (idx, iter) in u.iterations.as_deref().unwrap_or(&[]).iter().enumerate() {
            if iter.iter_type.as_deref() != Some("advisor_message") {
                continue;
            }
            let Some(adv_model) = iter
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "<synthetic>")
            else {
                continue;
            };
            let adv_usage = UsageAccumulator {
                input_tokens: iter.input_tokens.unwrap_or(0),
                cache_creation_input_tokens: iter.cache_creation_input_tokens.unwrap_or(0),
                cache_read_input_tokens: iter.cache_read_input_tokens.unwrap_or(0),
                output_tokens: iter.output_tokens.unwrap_or(0),
                reasoning_output_tokens: iter.reasoning_output_tokens.unwrap_or(0),
                cost_usd: 0.0,
            };
            if adv_usage.total_tokens() == 0 {
                continue;
            }
            let adv_1h = iter
                .cache_creation
                .as_ref()
                .and_then(|c| c.ephemeral_1h_input_tokens)
                .unwrap_or(0);
            let adv_hash = dedupe_base
                .as_deref()
                .map(|k| fnv1a_64(format!("{k}:advisor:{idx}").as_bytes()));
            out.push(ParsedRecord {
                event: UsageEvent {
                    timestamp,
                    source: SourceKind::Claude,
                    model: adv_model.to_string(),
                    session: session.clone(),
                    project: None,
                    file_path: String::new(),
                    usage: adv_usage,
                },
                pricing_ctx: PricingContext {
                    cache_creation_1h_tokens: adv_1h,
                    speed,
                    model_calls: 1,
                    reported_cost_usd: None,
                },
                dedupe_hash: adv_hash,
                used_unknown_pricing: false,
            });
        }
    }

    MultiLineResult::Parsed(out)
}

pub(super) fn parse_gemini_usage_line(line: &str, pricing: &PricingTable) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    // If type is present, allow "gemini" or step records that carry token metrics
    let entry_type = get_value(&value, "type").and_then(Value::as_str);
    if let Some(t) = entry_type {
        if t != "gemini"
            && get_value(&value, "tokens").is_none()
            && get_value(&value, "usage").is_none()
        {
            return ParseLineResult::MissingUsage;
        }
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };

    let model =
        extract_string(&value, &["model"]).unwrap_or_else(|| "gemini-3.6-flash".to_string());

    let usage = UsageAccumulator {
        input_tokens: extract_u64(&value, &["tokens.input"])
            .or_else(|| extract_u64(&value, &["usage.input_tokens"]))
            .or_else(|| extract_u64(&value, &["tokens.input_tokens"]))
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: extract_u64(&value, &["tokens.cached"])
            .or_else(|| extract_u64(&value, &["usage.cache_read_input_tokens"]))
            .unwrap_or(0),
        output_tokens: extract_u64(&value, &["tokens.output"])
            .or_else(|| extract_u64(&value, &["usage.output_tokens"]))
            .unwrap_or(0),
        reasoning_output_tokens: extract_u64(&value, &["tokens.thoughts"])
            .or_else(|| extract_u64(&value, &["usage.reasoning_output_tokens"]))
            .unwrap_or(0),
        cost_usd: 0.0,
    };

    let total_tokens = usage.total_tokens();
    if total_tokens == 0 {
        return ParseLineResult::MissingUsage;
    }

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };

    ParseLineResult::Parsed(ParsedLine {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Gemini,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
}

const GROK_COST_USD_TICKS_PER_DOLLAR: f64 = 1e10;

#[derive(Debug, Default, Deserialize)]
struct GrokModelUsageEntry {
    #[serde(
        default,
        rename = "inputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cachedReadTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cached_read_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheCreationTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_creation_tokens: Option<u64>,
    #[serde(
        default,
        rename = "outputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    output_tokens: Option<u64>,
    #[serde(
        default,
        rename = "reasoningTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    reasoning_tokens: Option<u64>,
    #[serde(
        default,
        rename = "costUsdTicks",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cost_usd_ticks: Option<u64>,
    #[serde(
        default,
        rename = "modelCalls",
        deserialize_with = "de_opt_u64_clamped"
    )]
    model_calls: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokTurnUsage {
    #[serde(
        default,
        rename = "inputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    input_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cachedReadTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cached_read_tokens: Option<u64>,
    #[serde(
        default,
        rename = "cacheCreationTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_creation_tokens: Option<u64>,
    #[serde(
        default,
        rename = "outputTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    output_tokens: Option<u64>,
    #[serde(
        default,
        rename = "reasoningTokens",
        deserialize_with = "de_opt_u64_clamped"
    )]
    reasoning_tokens: Option<u64>,
    #[serde(
        default,
        rename = "costUsdTicks",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cost_usd_ticks: Option<u64>,
    #[serde(
        default,
        rename = "modelCalls",
        deserialize_with = "de_opt_u64_clamped"
    )]
    model_calls: Option<u64>,
    #[serde(default, rename = "modelUsage")]
    model_usage: Option<HashMap<String, GrokModelUsageEntry>>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokUpdateObj {
    #[serde(default, rename = "sessionUpdate")]
    session_update: Option<String>,
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    usage: Option<GrokTurnUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokMetaObj {
    #[serde(default, rename = "agentTimestampMs")]
    agent_timestamp_ms: Option<RawTimestamp>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokParamsObj {
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    update: Option<GrokUpdateObj>,
    #[serde(default)]
    _meta: Option<GrokMetaObj>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokUnifiedCtx {
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    prompt_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cached_prompt_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    completion_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokLineRecord {
    #[serde(default)]
    timestamp: Option<RawTimestamp>,
    #[serde(default)]
    created_at: Option<RawTimestamp>,
    #[serde(default, rename = "createdAt")]
    created_at_camel: Option<RawTimestamp>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    params: Option<GrokParamsObj>,
    #[serde(default)]
    update: Option<GrokUpdateObj>,
    #[serde(default)]
    _meta: Option<GrokMetaObj>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    ts: Option<RawTimestamp>,
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    ctx: Option<GrokUnifiedCtx>,
}

fn grok_usage_from_raw(
    input: u64,
    cached_read: u64,
    cache_creation: u64,
    output: u64,
    reasoning: u64,
) -> UsageAccumulator {
    UsageAccumulator {
        input_tokens: input
            .saturating_sub(cached_read)
            .saturating_sub(cache_creation),
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cached_read,
        output_tokens: output,
        reasoning_output_tokens: reasoning,
        cost_usd: 0.0,
    }
}

/// Parse one line of Grok session (`updates.jsonl`) or unified log (`unified.jsonl`).
pub(super) fn parse_grok_usage_line_records(line: &str, pricing: &PricingTable) -> MultiLineResult {
    let rec: GrokLineRecord = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return MultiLineResult::InvalidJson,
    };

    let upd = rec
        .params
        .as_ref()
        .and_then(|p| p.update.as_ref())
        .or(rec.update.as_ref());

    if let Some(upd) = upd
        && upd.session_update.as_deref() == Some("turn_completed")
    {
        let timestamp = rec
            .params
            .as_ref()
            .and_then(|p| p._meta.as_ref())
            .and_then(|m| m.agent_timestamp_ms.as_ref())
            .and_then(RawTimestamp::to_utc)
            .or_else(|| {
                rec._meta
                    .as_ref()
                    .and_then(|m| m.agent_timestamp_ms.as_ref())
                    .and_then(RawTimestamp::to_utc)
            })
            .or_else(|| rec.timestamp.as_ref().and_then(RawTimestamp::to_utc))
            .or_else(|| rec.created_at.as_ref().and_then(RawTimestamp::to_utc))
            .or_else(|| rec.created_at_camel.as_ref().and_then(RawTimestamp::to_utc));
        let Some(timestamp) = timestamp else {
            return MultiLineResult::MissingUsage;
        };

        let session = rec
            .params
            .as_ref()
            .and_then(|p| p.session_id.clone())
            .or(rec.session_id)
            .unwrap_or_default();
        let Some(usage_val) = upd.usage.as_ref() else {
            return MultiLineResult::MissingUsage;
        };
        let prompt_id = upd.prompt_id.as_deref();
        let ts_ms = timestamp.timestamp_millis();
        let top_calls = usage_val.model_calls.unwrap_or(1).max(1) as u32;
        let top_cost_usd = usage_val
            .cost_usd_ticks
            .filter(|&t| t > 0)
            .map(|t| (t as f64) / GROK_COST_USD_TICKS_PER_DOLLAR);

        if let Some(model_usage) = usage_val.model_usage.as_ref()
            && !model_usage.is_empty()
        {
            let mut entries: Vec<(&str, UsageAccumulator, Option<f64>, u32)> =
                Vec::with_capacity(model_usage.len());
            let mut used_ticked_cost = 0.0;
            let mut unticked_tokens: u64 = 0;

            for (model_name, m) in model_usage {
                if model_name.is_empty() {
                    continue;
                }
                let u = grok_usage_from_raw(
                    m.input_tokens.unwrap_or(0),
                    m.cached_read_tokens.unwrap_or(0),
                    m.cache_creation_tokens.unwrap_or(0),
                    m.output_tokens.unwrap_or(0),
                    m.reasoning_tokens.unwrap_or(0),
                );
                let tot = u.total_tokens();
                if tot == 0 {
                    continue;
                }
                let rep = m
                    .cost_usd_ticks
                    .filter(|&t| t > 0)
                    .map(|t| (t as f64) / GROK_COST_USD_TICKS_PER_DOLLAR);
                if let Some(c) = rep {
                    used_ticked_cost += c;
                } else {
                    unticked_tokens += tot;
                }
                let calls = m.model_calls.map(|c| c.max(1) as u32).unwrap_or(top_calls);
                entries.push((model_name.as_str(), u, rep, calls));
            }

            if entries.is_empty() {
                return MultiLineResult::MissingUsage;
            }

            let remaining_cost = top_cost_usd.map(|c| (c - used_ticked_cost).max(0.0));
            let mut out = Vec::with_capacity(entries.len());
            for (model_name, mut u, mut rep, calls) in entries {
                if rep.is_none()
                    && let Some(rem) = remaining_cost
                    && unticked_tokens > 0
                {
                    rep = Some(rem * (u.total_tokens() as f64 / unticked_tokens as f64));
                }
                let ctx = PricingContext {
                    cache_creation_1h_tokens: 0,
                    speed: ServiceSpeed::Standard,
                    model_calls: calls,
                    reported_cost_usd: rep,
                };
                let (cost_usd, used_unknown_pricing) =
                    match pricing.estimate_cost_with_context(model_name, u, ctx) {
                        Some(v) => (v, false),
                        None => (0.0, true),
                    };
                u.cost_usd = cost_usd;
                let dedupe_hash = prompt_id.map(|pid| {
                    fnv1a_64(format!("{session}:{pid}:{ts_ms}:{model_name}").as_bytes())
                });
                out.push(ParsedRecord {
                    event: UsageEvent {
                        timestamp,
                        source: SourceKind::Grok,
                        model: model_name.to_string(),
                        session: session.clone(),
                        project: None,
                        file_path: String::new(),
                        usage: u,
                    },
                    pricing_ctx: ctx,
                    dedupe_hash,
                    used_unknown_pricing,
                });
            }
            return MultiLineResult::Parsed(out);
        }

        let mut u = grok_usage_from_raw(
            usage_val.input_tokens.unwrap_or(0),
            usage_val.cached_read_tokens.unwrap_or(0),
            usage_val.cache_creation_tokens.unwrap_or(0),
            usage_val.output_tokens.unwrap_or(0),
            usage_val.reasoning_tokens.unwrap_or(0),
        );
        if u.total_tokens() == 0 {
            return MultiLineResult::MissingUsage;
        }
        let model = usage_val
            .model
            .clone()
            .or_else(|| usage_val.model_id.clone())
            .unwrap_or_else(|| crate::types::GROK_DEFAULT_MODEL_FALLBACK.to_string());
        let ctx = PricingContext {
            cache_creation_1h_tokens: 0,
            speed: ServiceSpeed::Standard,
            model_calls: top_calls,
            reported_cost_usd: top_cost_usd,
        };
        let (cost_usd, used_unknown_pricing) =
            match pricing.estimate_cost_with_context(&model, u, ctx) {
                Some(v) => (v, false),
                None => (0.0, true),
            };
        u.cost_usd = cost_usd;
        let dedupe_hash =
            prompt_id.map(|pid| fnv1a_64(format!("{session}:{pid}:{ts_ms}:{model}").as_bytes()));
        return MultiLineResult::Parsed(vec![ParsedRecord {
            event: UsageEvent {
                timestamp,
                source: SourceKind::Grok,
                model,
                session,
                project: None,
                file_path: String::new(),
                usage: u,
            },
            pricing_ctx: ctx,
            dedupe_hash,
            used_unknown_pricing,
        }]);
    }

    // Format 2: unified.jsonl line with shell.turn.inference_done
    if rec.msg.as_deref() != Some("shell.turn.inference_done") {
        return MultiLineResult::MissingUsage;
    }

    let Some(timestamp) = rec.ts.as_ref().and_then(RawTimestamp::to_utc) else {
        return MultiLineResult::MissingUsage;
    };
    let Some(session) = rec.sid else {
        return MultiLineResult::MissingUsage;
    };
    let Some(ctx_obj) = rec.ctx else {
        return MultiLineResult::MissingUsage;
    };

    let prompt_tokens = ctx_obj.prompt_tokens.unwrap_or(0);
    let cached_prompt_tokens = ctx_obj.cached_prompt_tokens.unwrap_or(0);
    let completion_tokens = ctx_obj.completion_tokens.unwrap_or(0);
    let reasoning_tokens = ctx_obj.reasoning_tokens.unwrap_or(0);

    let mut usage = UsageAccumulator {
        input_tokens: prompt_tokens.saturating_sub(cached_prompt_tokens),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached_prompt_tokens,
        output_tokens: completion_tokens,
        reasoning_output_tokens: reasoning_tokens,
        cost_usd: 0.0,
    };

    if usage.total_tokens() == 0 {
        return MultiLineResult::MissingUsage;
    }

    let model = crate::types::GROK_DEFAULT_MODEL_FALLBACK.to_string();
    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };
    usage.cost_usd = cost_usd;

    MultiLineResult::Parsed(vec![ParsedRecord {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Grok,
            model,
            session,
            project: None,
            file_path: String::new(),
            usage,
        },
        pricing_ctx: PricingContext::default(),
        dedupe_hash: None,
        used_unknown_pricing,
    }])
}

#[derive(Debug, Default, Deserialize)]
struct PiCostObj {
    #[serde(default)]
    total: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct PiUsageObj {
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    input: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    output: Option<u64>,
    #[serde(default, rename = "cacheRead", deserialize_with = "de_opt_u64_clamped")]
    cache_read: Option<u64>,
    #[serde(
        default,
        rename = "cacheWrite",
        deserialize_with = "de_opt_u64_clamped"
    )]
    cache_write: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning: Option<u64>,
    #[serde(default)]
    cost: Option<PiCostObj>,
}

#[derive(Debug, Default, Deserialize)]
struct PiMessageObj {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "responseModel")]
    response_model: Option<String>,
    #[serde(default)]
    usage: Option<PiUsageObj>,
}

#[derive(Debug, Default, Deserialize)]
struct PiLineRecord {
    #[serde(rename = "type", default)]
    entry_type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<PiUsageObj>,
    #[serde(default)]
    message: Option<PiMessageObj>,
}

/// Parse one JSONL record from a Pi coding agent session (`~/.pi/agent/sessions`).
pub(super) fn parse_pi_usage_line_records(
    line: &str,
    session_id: &mut String,
    pricing: &PricingTable,
) -> MultiLineResult {
    let rec: PiLineRecord = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return MultiLineResult::InvalidJson,
    };

    let Some(entry_type) = rec.entry_type.as_deref() else {
        return MultiLineResult::MissingUsage;
    };

    if entry_type == "session" {
        if let Some(id) = rec.id.filter(|s| !s.is_empty()) {
            *session_id = id;
        }
        return MultiLineResult::MissingUsage;
    }

    let Some(id) = rec.id.as_deref().filter(|s| !s.is_empty()) else {
        return MultiLineResult::MissingUsage;
    };
    let Some(ts_raw) = rec.timestamp.as_deref() else {
        return MultiLineResult::MissingUsage;
    };
    let Some(timestamp) = DateTime::parse_from_rfc3339(ts_raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
    else {
        return MultiLineResult::MissingUsage;
    };

    let (model, usage_obj) = match entry_type {
        "message" => {
            let Some(msg) = rec.message else {
                return MultiLineResult::MissingUsage;
            };
            match msg.role.as_deref() {
                Some("assistant") => {
                    let Some(prov) = msg.provider.as_deref().filter(|s| !s.is_empty()) else {
                        return MultiLineResult::MissingUsage;
                    };
                    let Some(rate_model) = msg
                        .response_model
                        .as_deref()
                        .or(msg.model.as_deref())
                        .filter(|s| !s.is_empty())
                    else {
                        return MultiLineResult::MissingUsage;
                    };
                    let Some(u) = msg.usage else {
                        return MultiLineResult::MissingUsage;
                    };
                    (format!("{prov}/{rate_model}"), u)
                }
                Some("toolResult") => {
                    let Some(u) = msg.usage else {
                        return MultiLineResult::MissingUsage;
                    };
                    ("Tools/summaries".to_string(), u)
                }
                _ => return MultiLineResult::MissingUsage,
            }
        }
        "usage" => {
            let Some(prov) = rec.provider.as_deref().filter(|s| !s.is_empty()) else {
                return MultiLineResult::MissingUsage;
            };
            let Some(rate_model) = rec.model.as_deref().filter(|s| !s.is_empty()) else {
                return MultiLineResult::MissingUsage;
            };
            let Some(u) = rec.usage else {
                return MultiLineResult::MissingUsage;
            };
            (format!("{prov}/{rate_model}"), u)
        }
        "compaction" | "branch_summary" => {
            let Some(u) = rec.usage else {
                return MultiLineResult::MissingUsage;
            };
            ("Tools/summaries".to_string(), u)
        }
        _ => return MultiLineResult::MissingUsage,
    };

    let output_tokens = usage_obj.output.unwrap_or(0);
    let mut usage = UsageAccumulator {
        input_tokens: usage_obj.input.unwrap_or(0),
        cache_creation_input_tokens: usage_obj.cache_write.unwrap_or(0),
        cache_read_input_tokens: usage_obj.cache_read.unwrap_or(0),
        output_tokens,
        reasoning_output_tokens: output_tokens.min(usage_obj.reasoning.unwrap_or(0)),
        cost_usd: 0.0,
    };
    let reported_cost_usd = usage_obj
        .cost
        .and_then(|c| c.total)
        .filter(|c| c.is_finite() && *c > 0.0);

    if usage.total_tokens() == 0 && reported_cost_usd.is_none() {
        return MultiLineResult::MissingUsage;
    }

    let ctx = PricingContext {
        reported_cost_usd,
        ..PricingContext::default()
    };
    let (cost_usd, used_unknown_pricing) =
        match pricing.estimate_cost_with_context(&model, usage, ctx) {
            Some(v) => (v, false),
            None => (0.0, true),
        };
    usage.cost_usd = cost_usd;

    let dedupe_hash = Some(fnv1a_64(format!("pi\0{id}\0{ts_raw}\0{model}").as_bytes()));
    MultiLineResult::Parsed(vec![ParsedRecord {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Pi,
            model,
            session: session_id.clone(),
            project: None,
            file_path: String::new(),
            usage,
        },
        pricing_ctx: ctx,
        dedupe_hash,
        used_unknown_pricing,
    }])
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MuseModelRef {
    Name(String),
    Obj { model_id: String },
}

#[derive(Debug, Default, Deserialize)]
struct MuseUsageObj {
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cached_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cached_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_read_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    cache_write_tokens: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64_clamped")]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct MuseEventObj {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    model: Option<MuseModelRef>,
    #[serde(default)]
    usage: Option<MuseUsageObj>,
}

#[derive(Debug, Default, Deserialize)]
struct MusePayloadObj {
    #[serde(default)]
    source_run_record_id: Option<String>,
    #[serde(default)]
    event: Option<MuseEventObj>,
}

#[derive(Debug, Default, Deserialize)]
struct MuseStreamObj {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MuseLineRecord {
    #[serde(default)]
    schema_version: Option<i64>,
    #[serde(default)]
    record_type: Option<String>,
    #[serde(default)]
    payload_type: Option<String>,
    #[serde(default)]
    payload_schema_version: Option<i64>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    recorded_at: Option<serde_json::Number>,
    #[serde(default)]
    stream: Option<MuseStreamObj>,
    #[serde(default)]
    payload: Option<MusePayloadObj>,
}

/// Parse one JSONL record from a Muse Code session (`$MUSE_HOME/sessions/**/session.jsonl`).
pub(super) fn parse_muse_usage_line_records(line: &str, pricing: &PricingTable) -> MultiLineResult {
    let rec: MuseLineRecord = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return MultiLineResult::InvalidJson,
    };

    if rec.schema_version != Some(1)
        || rec.record_type.as_deref() != Some("event")
        || rec.payload_type.as_deref() != Some("runtime.session")
        || !matches!(rec.payload_schema_version, Some(1 | 3))
    {
        return MultiLineResult::MissingUsage;
    }

    let Some(id) = rec.id.as_deref().filter(|s| !s.is_empty()) else {
        return MultiLineResult::MissingUsage;
    };
    let Some(recorded_us) = rec.recorded_at.and_then(|n| {
        n.as_i64().filter(|&v| v >= 0).or_else(|| {
            n.as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.trunc() as i64)
        })
    }) else {
        return MultiLineResult::MissingUsage;
    };
    let Some(timestamp) = DateTime::from_timestamp_millis(recorded_us / 1_000) else {
        return MultiLineResult::MissingUsage;
    };

    let Some(stream) = rec.stream else {
        return MultiLineResult::MissingUsage;
    };
    if stream.kind.as_deref() != Some("session") {
        return MultiLineResult::MissingUsage;
    }
    let Some(session) = stream.id.filter(|s| !s.is_empty()) else {
        return MultiLineResult::MissingUsage;
    };

    let Some(payload) = rec.payload else {
        return MultiLineResult::MissingUsage;
    };
    let Some(ev) = payload.event else {
        return MultiLineResult::MissingUsage;
    };
    if !matches!(
        ev.kind.as_deref(),
        Some("model_completed" | "automated_review_completed")
    ) {
        return MultiLineResult::MissingUsage;
    }
    let model = match ev.model {
        Some(MuseModelRef::Name(s)) if !s.is_empty() => s,
        Some(MuseModelRef::Obj { model_id }) if !model_id.is_empty() => model_id,
        _ => return MultiLineResult::MissingUsage,
    };
    let Some(u) = ev.usage else {
        return MultiLineResult::MissingUsage;
    };

    let input = u.input_tokens.unwrap_or(0);
    let cached_read = input.min(
        u.cache_read_tokens
            .or(u.cached_input_tokens)
            .or(u.cached_tokens)
            .unwrap_or(0),
    );
    let cache_write = (input - cached_read).min(u.cache_write_tokens.unwrap_or(0));
    let output_tokens = u.output_tokens.unwrap_or(0);

    let mut usage = UsageAccumulator {
        input_tokens: input - cached_read - cache_write,
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cached_read,
        output_tokens,
        reasoning_output_tokens: output_tokens.min(u.reasoning_tokens.unwrap_or(0)),
        cost_usd: 0.0,
    };
    if usage.total_tokens() == 0 {
        return MultiLineResult::MissingUsage;
    }

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };
    usage.cost_usd = cost_usd;

    let dedupe_id = payload
        .source_run_record_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    let dedupe_hash = Some(fnv1a_64(format!("muse\0{dedupe_id}").as_bytes()));

    MultiLineResult::Parsed(vec![ParsedRecord {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Muse,
            model,
            session,
            project: None,
            file_path: String::new(),
            usage,
        },
        pricing_ctx: PricingContext::default(),
        dedupe_hash,
        used_unknown_pricing,
    }])
}

fn parse_opencode_timestamp_ms(value: &Value) -> Option<i64> {
    let created = get_value(value, "time.created").and_then(Value::as_i64);
    let completed = get_value(value, "time.completed").and_then(Value::as_i64);
    completed.or(created)
}

fn opencode_project_from_root(value: &Value) -> Option<String> {
    let root = get_value(value, "path.root")
        .and_then(Value::as_str)
        .or_else(|| get_value(value, "path.cwd").and_then(Value::as_str))?;
    Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

fn parse_opencode_model(value: &Value) -> Option<String> {
    extract_string(value, &["model.modelID", "modelID", "model"])
}

fn parse_opencode_usage(value: &Value) -> Option<UsageAccumulator> {
    let input_tokens = extract_u64(value, &["tokens.input"])?;
    let output_tokens = extract_u64(value, &["tokens.output"]).unwrap_or(0);
    let reasoning_output_tokens = extract_u64(value, &["tokens.reasoning"]).unwrap_or(0);
    let cache_read = extract_u64(value, &["tokens.cache.read"]).unwrap_or(0);
    let cache_write = extract_u64(value, &["tokens.cache.write"]).unwrap_or(0);
    Some(UsageAccumulator {
        input_tokens,
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cache_read,
        output_tokens,
        reasoning_output_tokens,
        cost_usd: 0.0,
    })
}

fn parse_opencode_message_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    cache_enabled: bool,
) -> Option<ParsedFileOutput> {
    let bytes = std::fs::read(&job.file.path).ok()?;
    if bytes.len() > MAX_JSON_LINE_BYTES {
        stats.lines_total.fetch_add(1, Ordering::Relaxed);
        stats.lines_invalid_json.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;

    stats.lines_total.fetch_add(1, Ordering::Relaxed);

    let Some(ts_ms) = parse_opencode_timestamp_ms(&value) else {
        stats.lines_missing_usage.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let Some(timestamp) = DateTime::from_timestamp_millis(ts_ms) else {
        stats.lines_missing_usage.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    if !date_filter_allows(filter, timestamp, timezone) {
        stats.lines_filtered.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let Some(model) = parse_opencode_model(&value) else {
        stats.lines_missing_usage.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let Some(usage) = parse_opencode_usage(&value) else {
        stats.lines_missing_usage.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if usage.total_tokens() == 0 {
        stats.lines_missing_usage.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };
    if used_unknown_pricing {
        stats.lines_unknown_pricing.fetch_add(1, Ordering::Relaxed);
    }

    let session = extract_string(&value, &["sessionID", "sessionId", "session_id"])
        .or_else(|| {
            // Legacy OpenCode stores messages as:
            //   .../storage/message/<session_id>/<message_id>.json
            // Some exports omit the session id field in the JSON blob.
            let mut found = None;
            for (idx, component) in job.file.path.components().enumerate() {
                if component.as_os_str() == "message" {
                    found = job
                        .file
                        .path
                        .components()
                        .nth(idx + 1)
                        .map(|c| c.as_os_str().to_string_lossy().to_string());
                    break;
                }
            }
            found
        })
        .or_else(|| {
            job.file
                .path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    let project = opencode_project_from_root(&value);

    stats.lines_parsed.fetch_add(1, Ordering::Relaxed);

    let file_path = job.file.path.display().to_string();
    let event = UsageEvent {
        timestamp,
        source: SourceKind::OpenCode,
        model,
        session,
        project,
        file_path: file_path.clone(),
        usage: UsageAccumulator { cost_usd, ..usage },
    };

    let cached_events = if cache_enabled {
        vec![cached_usage_event_for_file(
            &event,
            PricingContext::default(),
            None,
            &file_path,
        )]
    } else {
        Vec::new()
    };
    Some(ParsedFileOutput {
        events: vec![InternalEvent {
            event,
            pricing_ctx: PricingContext::default(),
            dedupe_hash: None,
        }],
        cache_entry: CachedFileEntry {
            fingerprint: job.fingerprint,
            stats: CachedFileStats {
                lines_total: 1,
                lines_invalid_json: 0,
                lines_missing_usage: 0,
                lines_unknown_pricing: if used_unknown_pricing { 1 } else { 0 },
            },
            events: cached_events,
            parsed_offset: job.fingerprint.size,
            ..CachedFileEntry::default()
        },
    })
}

fn decode_varint(data: &[u8], mut offset: usize) -> Option<(u64, usize)> {
    let mut res: u64 = 0;
    let mut shift = 0;
    while offset < data.len() {
        let b = data[offset];
        let val = (b & 0x7f) as u64;
        if shift >= 64 || (shift == 63 && val > 1) {
            return None;
        }
        res |= val << shift;
        offset += 1;
        if (b & 0x80) == 0 {
            return Some((res, offset));
        }
        shift += 7;
    }
    None
}

enum ProtoWireVal<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed64,
    Fixed32,
}

fn parse_proto_fields<'a>(data: &'a [u8]) -> Vec<(u32, ProtoWireVal<'a>)> {
    let mut offset = 0;
    let mut fields = Vec::new();
    while offset < data.len() {
        let Some((tag_byte, new_off)) = decode_varint(data, offset) else {
            break;
        };
        offset = new_off;
        let field_number = (tag_byte >> 3) as u32;
        let wire_type = (tag_byte & 0x07) as u32;

        match wire_type {
            0 => {
                let Some((val, new_off)) = decode_varint(data, offset) else {
                    break;
                };
                offset = new_off;
                fields.push((field_number, ProtoWireVal::Varint(val)));
            }
            2 => {
                let Some((length, new_off)) = decode_varint(data, offset) else {
                    break;
                };
                let len = length as usize;
                if new_off + len > data.len() {
                    break;
                }
                let val_bytes = &data[new_off..new_off + len];
                offset = new_off + len;
                fields.push((field_number, ProtoWireVal::Bytes(val_bytes)));
            }
            1 => {
                if offset + 8 > data.len() {
                    break;
                }
                offset += 8;
                fields.push((field_number, ProtoWireVal::Fixed64));
            }
            5 => {
                if offset + 4 > data.len() {
                    break;
                }
                offset += 4;
                fields.push((field_number, ProtoWireVal::Fixed32));
            }
            _ => break,
        }
    }
    fields
}

struct ParsedAntigravityGen {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    timestamp_secs: i64,
}

/// The bytes payload of the first length-delimited (wire type 2) field with
/// the given field number, or `None` if absent or a different wire type.
fn find_proto_bytes<'a>(fields: &[(u32, ProtoWireVal<'a>)], field_number: u32) -> Option<&'a [u8]> {
    fields.iter().find_map(|(fn_num, val)| match val {
        ProtoWireVal::Bytes(b) if *fn_num == field_number => Some(*b),
        _ => None,
    })
}

/// The value of the first varint (wire type 0) field with the given field
/// number, or `None` if absent or a different wire type.
fn find_proto_varint(fields: &[(u32, ProtoWireVal<'_>)], field_number: u32) -> Option<u64> {
    fields.iter().find_map(|(fn_num, val)| match val {
        ProtoWireVal::Varint(v) if *fn_num == field_number => Some(*v),
        _ => None,
    })
}

/// Extracts one generation event from a `gen_metadata.data` protobuf blob.
///
/// Antigravity has no published `.proto` schema, so these field numbers
/// were reverse-engineered from real blobs. Assumed message shape (numbers
/// are field numbers, nesting shows length-delimited submessages):
///
/// ```text
/// 1: bytes   generation record wrapper
///    4: bytes   token usage
///       2: varint  input_tokens
///       3: varint  output_tokens
///       5: varint  cache_read_tokens
///    9: bytes   request timing
///       4: bytes   wall-clock timestamp
///          1: varint  unix_epoch_seconds
///    19: string  model_id (e.g. "gemini-3.6-flash")
/// ```
///
/// If Antigravity's internal schema changes, a renumbered field could
/// coincidentally decode as a plausible varint/string rather than failing
/// loudly — the only safety net is the all-zero-tokens rejection below.
/// Re-verify against a fresh blob dump if costs/tokens look wrong after an
/// Antigravity app update.
fn extract_antigravity_gen_event(blob: &[u8]) -> Option<ParsedAntigravityGen> {
    let top = parse_proto_fields(blob);
    let f1 = parse_proto_fields(find_proto_bytes(&top, 1)?);

    let model = find_proto_bytes(&f1, 19)
        .and_then(|b| std::str::from_utf8(b).ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("gemini-3.6-flash")
        .to_string();

    let usage = find_proto_bytes(&f1, 4)
        .map(parse_proto_fields)
        .unwrap_or_default();
    let input_tokens = find_proto_varint(&usage, 2).unwrap_or(0);
    let output_tokens = find_proto_varint(&usage, 3).unwrap_or(0);
    let cache_read_tokens = find_proto_varint(&usage, 5).unwrap_or(0);

    let timestamp_secs = find_proto_bytes(&f1, 9)
        .map(parse_proto_fields)
        .and_then(|timing| find_proto_bytes(&timing, 4))
        .map(parse_proto_fields)
        .and_then(|wall_clock| find_proto_varint(&wall_clock, 1))
        .map(|v| v as i64)
        .unwrap_or(0);

    if input_tokens == 0 && output_tokens == 0 && cache_read_tokens == 0 {
        return None;
    }

    Some(ParsedAntigravityGen {
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        timestamp_secs,
    })
}

fn parse_antigravity_db_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    cache_enabled: bool,
) -> Option<ParsedFileOutput> {
    let path = &job.file.path;

    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|_| {
        stats.files_open_failed.fetch_add(1, Ordering::Relaxed);
    })
    .ok()?;

    let file_mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(DateTime::<Utc>::from);

    let mut stmt = conn.prepare("SELECT data FROM gen_metadata").ok()?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)).ok()?;

    let mut events = Vec::new();
    let mut cached_events = Vec::new();
    let mut lines_total = 0usize;
    let mut lines_parsed = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;

    for row_res in rows {
        let Ok(blob) = row_res else { continue };
        lines_total += 1;

        let Some(gen_event) = extract_antigravity_gen_event(&blob) else {
            lines_missing_usage += 1;
            continue;
        };

        let timestamp = if gen_event.timestamp_secs > 10_000_000_000 {
            DateTime::from_timestamp_millis(gen_event.timestamp_secs)
        } else if gen_event.timestamp_secs > 0 {
            DateTime::from_timestamp(gen_event.timestamp_secs, 0)
        } else {
            file_mtime
        };
        let Some(timestamp) = timestamp else {
            lines_missing_usage += 1;
            continue;
        };

        let usage = UsageAccumulator {
            input_tokens: gen_event.input_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: gen_event.cache_read_tokens,
            output_tokens: gen_event.output_tokens,
            reasoning_output_tokens: 0,
            cost_usd: 0.0,
        };

        let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&gen_event.model, usage)
        {
            Some(v) => (v, false),
            None => (0.0, true),
        };
        if used_unknown_pricing {
            lines_unknown_pricing += 1;
        }

        let file_path_str = path.to_string_lossy().to_string();
        let event = UsageEvent {
            timestamp,
            source: SourceKind::Gemini,
            model: gen_event.model,
            session: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            project: None,
            file_path: file_path_str.clone(),
            usage: UsageAccumulator { cost_usd, ..usage },
        };
        if cache_enabled {
            cached_events.push(cached_usage_event_for_file(
                &event,
                PricingContext::default(),
                None,
                &file_path_str,
            ));
        }

        if !date_filter_allows(filter, timestamp, timezone) {
            lines_filtered += 1;
            continue;
        }

        events.push(InternalEvent {
            event,
            pricing_ctx: PricingContext::default(),
            dedupe_hash: None,
        });
        lines_parsed += 1;
    }

    stats.lines_total.fetch_add(lines_total, Ordering::Relaxed);
    stats
        .lines_parsed
        .fetch_add(lines_parsed, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(lines_missing_usage, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(lines_unknown_pricing, Ordering::Relaxed);
    stats
        .lines_filtered
        .fetch_add(lines_filtered, Ordering::Relaxed);

    Some(ParsedFileOutput {
        events,
        cache_entry: CachedFileEntry {
            fingerprint: job.fingerprint,
            stats: CachedFileStats {
                lines_total,
                lines_invalid_json: 0,
                lines_missing_usage,
                lines_unknown_pricing,
            },
            events: cached_events,
            parsed_offset: job.fingerprint.size,
            ..CachedFileEntry::default()
        },
    })
}

fn parse_opencode_db_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    cache_enabled: bool,
) -> Option<ParsedFileOutput> {
    let path = &job.file.path;

    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|_| {
        stats.files_open_failed.fetch_add(1, Ordering::Relaxed);
    })
    .ok()?;

    let mut events = Vec::new();
    let mut cached_events = Vec::new();

    let sql = "SELECT m.id, m.session_id, s.id, m.time_created, m.data, s.directory, p.worktree \
               FROM message m \
               LEFT JOIN session s ON m.session_id = s.id \
               LEFT JOIN project p ON s.project_id = p.id \
               ORDER BY m.time_created ASC";

    let mut lines_total = 0usize;
    let mut lines_parsed = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;

    let mut stmt = conn.prepare(sql).ok()?;
    let mut rows = stmt.query([]).ok()?;

    while let Ok(Some(row)) = rows.next() {
        lines_total += 1;
        let message_id: String = row.get(0).ok()?;
        let msg_session_id: String = row
            .get::<_, String>(1)
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| row.get::<_, i64>(1).ok().map(|v| v.to_string()))
            .unwrap_or_default();
        let session_id: String = row
            .get::<_, String>(2)
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| row.get::<_, i64>(2).ok().map(|v| v.to_string()))
            .unwrap_or_default();
        let time_created: i64 = row.get(3).ok()?;
        let data: String = row.get(4).ok()?;
        if data.len() > MAX_JSON_LINE_BYTES {
            lines_invalid_json += 1;
            continue;
        }
        let session_dir: Option<String> = row.get(5).ok();
        let project_worktree: Option<String> = row.get(6).ok();

        let Some(timestamp) = DateTime::from_timestamp_millis(time_created) else {
            lines_missing_usage += 1;
            continue;
        };

        let value: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => {
                lines_invalid_json += 1;
                continue;
            }
        };

        // Only assistant messages have token accounting.
        if get_value(&value, "role")
            .and_then(Value::as_str)
            .is_some_and(|r| r != "assistant")
        {
            lines_missing_usage += 1;
            continue;
        }

        let Some(model) = parse_opencode_model(&value) else {
            lines_missing_usage += 1;
            continue;
        };
        let Some(usage) = parse_opencode_usage(&value) else {
            lines_missing_usage += 1;
            continue;
        };
        if usage.total_tokens() == 0 {
            lines_missing_usage += 1;
            continue;
        }

        let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
            Some(v) => (v, false),
            None => (0.0, true),
        };
        if used_unknown_pricing {
            lines_unknown_pricing += 1;
        }

        let mut project = session_dir
            .as_deref()
            .and_then(|d| Path::new(d).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .or_else(|| {
                project_worktree
                    .as_deref()
                    .and_then(|w| Path::new(w).file_name())
                    .map(|n| n.to_string_lossy().to_string())
            });

        let session = if !msg_session_id.is_empty() {
            msg_session_id
        } else if !session_id.is_empty() {
            session_id
        } else {
            session_dir
                .as_deref()
                .and_then(|d| Path::new(d).file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        if project.is_none() && !session.is_empty() {
            project = Some(session.clone());
        }

        let file_path_str = path.display().to_string();
        let event = UsageEvent {
            timestamp,
            source: SourceKind::OpenCode,
            model: model.clone(),
            session,
            project,
            file_path: format!("{file_path_str}#{message_id}"),
            usage: UsageAccumulator { cost_usd, ..usage },
        };
        if cache_enabled {
            cached_events.push(cached_usage_event_for_file(
                &event,
                PricingContext::default(),
                None,
                &file_path_str,
            ));
        }

        if !date_filter_allows(filter, timestamp, timezone) {
            lines_filtered += 1;
            continue;
        }

        events.push(InternalEvent {
            event,
            pricing_ctx: PricingContext::default(),
            dedupe_hash: None,
        });
        lines_parsed += 1;
    }

    stats.lines_total.fetch_add(lines_total, Ordering::Relaxed);
    stats
        .lines_parsed
        .fetch_add(lines_parsed, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(lines_missing_usage, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(lines_unknown_pricing, Ordering::Relaxed);
    stats
        .lines_filtered
        .fetch_add(lines_filtered, Ordering::Relaxed);

    Some(ParsedFileOutput {
        events,
        cache_entry: CachedFileEntry {
            fingerprint: job.fingerprint,
            stats: CachedFileStats {
                lines_total,
                lines_invalid_json,
                lines_missing_usage,
                lines_unknown_pricing,
            },
            events: cached_events,
            parsed_offset: job.fingerprint.size,
            ..CachedFileEntry::default()
        },
    })
}

fn codex_speed_from_tier(tier: Option<&str>) -> ServiceSpeed {
    match tier {
        Some("priority") | Some("fast") => ServiceSpeed::Fast,
        Some("ultrafast") => ServiceSpeed::Ultrafast,
        _ => ServiceSpeed::Standard,
    }
}

pub(super) fn parse_codex_usage_line_records(
    line: &str,
    state: &mut CodexParseState,
    pricing: &PricingTable,
) -> MultiLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return MultiLineResult::InvalidJson,
    };

    let Some(entry_type) = get_value(&value, "type").and_then(|v| v.as_str()) else {
        return MultiLineResult::MissingUsage;
    };

    if entry_type == "session_meta" {
        if let Some(payload) = get_value(&value, "payload")
            && let Some(forked) = payload.get("forked_from_id").and_then(Value::as_str)
            && !forked.is_empty()
        {
            state.fork_anchor_ts = extract_timestamp(&value);
        }
        return MultiLineResult::MissingUsage;
    }

    if entry_type == "turn_context" {
        state.request_ledger_active = false;
        if let Some(payload) = get_value(&value, "payload")
            && let Some(model) = extract_codex_model(payload)
        {
            state.current_model = Some(model);
            state.current_model_is_fallback = false;
        }
        return MultiLineResult::MissingUsage;
    }

    if entry_type == "token_usage_record" {
        let Some(timestamp) = extract_timestamp(&value) else {
            return MultiLineResult::MissingUsage;
        };
        if state
            .fork_anchor_ts
            .is_some_and(|anchor| timestamp <= anchor)
        {
            return MultiLineResult::MissingUsage;
        }
        let Some(payload) = get_value(&value, "payload") else {
            return MultiLineResult::MissingUsage;
        };
        let Some(usage_val) = payload.get("usage") else {
            return MultiLineResult::MissingUsage;
        };
        let Some(raw) = parse_codex_raw_usage(usage_val) else {
            return MultiLineResult::MissingUsage;
        };
        if raw.is_zero() {
            return MultiLineResult::MissingUsage;
        }
        let cache_write = extract_u64(usage_val, &["cache_write_input_tokens"]).unwrap_or(0);
        if payload
            .get("response_id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            state.request_ledger_active = true;
        }
        let model = extract_codex_model(payload)
            .or_else(|| state.current_model.clone())
            .unwrap_or_else(|| LEGACY_CODEX_FALLBACK_MODEL.to_string());
        let mut usage = UsageAccumulator {
            input_tokens: raw
                .input_tokens
                .saturating_sub(raw.cached_input_tokens)
                .saturating_sub(cache_write),
            cache_creation_input_tokens: cache_write,
            cache_read_input_tokens: raw.cached_input_tokens,
            output_tokens: raw.output_tokens,
            reasoning_output_tokens: raw.reasoning_output_tokens,
            cost_usd: 0.0,
        };
        let ctx = PricingContext {
            speed: state.service_tier,
            ..PricingContext::default()
        };
        let (cost_usd, used_unknown_pricing) =
            match pricing.estimate_cost_with_context(&model, usage, ctx) {
                Some(v) => (v, false),
                None => (0.0, true),
            };
        usage.cost_usd = cost_usd;
        return MultiLineResult::Parsed(vec![ParsedRecord {
            event: UsageEvent {
                timestamp,
                source: SourceKind::Codex,
                model,
                session: String::new(),
                project: None,
                file_path: String::new(),
                usage,
            },
            pricing_ctx: ctx,
            dedupe_hash: None,
            used_unknown_pricing,
        }]);
    }

    if entry_type != "event_msg" {
        return MultiLineResult::MissingUsage;
    }

    let Some(payload_type) = get_value(&value, "payload.type").and_then(|v| v.as_str()) else {
        return MultiLineResult::MissingUsage;
    };

    if payload_type == "thread_settings_applied" {
        let tier = get_value(&value, "payload.thread_settings.service_tier")
            .or_else(|| get_value(&value, "payload.service_tier"))
            .and_then(Value::as_str);
        state.service_tier = codex_speed_from_tier(tier);
        return MultiLineResult::MissingUsage;
    }

    if payload_type != "token_count" {
        return MultiLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return MultiLineResult::MissingUsage;
    };

    let info = get_value(&value, "payload.info");
    let last_usage = info.and_then(|v| get_value(v, "last_token_usage"));
    let total_usage = info.and_then(|v| get_value(v, "total_token_usage"));

    let parsed_last = last_usage.and_then(parse_codex_raw_usage);
    let parsed_total = total_usage.and_then(parse_codex_raw_usage);

    if state
        .fork_anchor_ts
        .is_some_and(|anchor| timestamp <= anchor)
    {
        if let Some(total) = parsed_total {
            state.previous_totals = Some(total);
        }
        return MultiLineResult::MissingUsage;
    }

    if state.request_ledger_active {
        if let Some(total) = parsed_total {
            state.previous_totals = Some(total);
        }
        return MultiLineResult::MissingUsage;
    }

    // Codex periodically re-emits identical cumulative token_count snapshots with
    // the previous last_token_usage still populated. Skip when total_token_usage
    // has not advanced.
    if let (Some(total), Some(prev)) = (parsed_total, state.previous_totals)
        && total.input_tokens == prev.input_tokens
        && total.cached_input_tokens == prev.cached_input_tokens
        && total.output_tokens == prev.output_tokens
        && total.reasoning_output_tokens == prev.reasoning_output_tokens
    {
        return MultiLineResult::MissingUsage;
    }

    let raw_delta = if let Some(last) = parsed_last {
        if parsed_total.is_none() {
            let sig = (
                last.input_tokens,
                last.cached_input_tokens,
                last.output_tokens,
                last.reasoning_output_tokens,
                last.total_tokens,
            );
            if state.last_usage_signature == Some(sig) {
                return MultiLineResult::MissingUsage;
            }
            state.last_usage_signature = Some(sig);
        }
        Some(last)
    } else if let Some(total) = parsed_total {
        Some(subtract_codex_raw_usage(total, state.previous_totals))
    } else {
        None
    };

    if let Some(total) = parsed_total {
        state.previous_totals = Some(total);
    }

    let Some(raw_delta) = raw_delta else {
        return MultiLineResult::MissingUsage;
    };
    if raw_delta.is_zero() {
        return MultiLineResult::MissingUsage;
    }

    let extracted_model = get_value(&value, "payload").and_then(extract_codex_model);
    if let Some(model) = extracted_model.as_ref() {
        state.current_model = Some(model.clone());
        state.current_model_is_fallback = false;
    }

    let model = if let Some(model) = extracted_model {
        model
    } else if let Some(model) = state.current_model.clone() {
        model
    } else {
        state.current_model = Some(LEGACY_CODEX_FALLBACK_MODEL.to_string());
        state.current_model_is_fallback = true;
        LEGACY_CODEX_FALLBACK_MODEL.to_string()
    };

    let mut usage = codex_delta_to_usage(raw_delta);
    let ctx = PricingContext {
        speed: state.service_tier,
        ..PricingContext::default()
    };

    let (cost_usd, used_unknown_pricing) =
        match pricing.estimate_cost_with_context(&model, usage, ctx) {
            Some(v) => (v, false),
            None => (0.0, true),
        };
    usage.cost_usd = cost_usd;

    MultiLineResult::Parsed(vec![ParsedRecord {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Codex,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage,
        },
        pricing_ctx: ctx,
        dedupe_hash: None,
        used_unknown_pricing,
    }])
}

pub(super) fn extract_codex_model(value: &Value) -> Option<String> {
    extract_string(
        value,
        &[
            "model",
            "payload.model",
            "info.model",
            "info.current_model",
            "current_model",
        ],
    )
}

pub(super) fn parse_codex_raw_usage(value: &Value) -> Option<CodexRawUsage> {
    let input_tokens = extract_u64(value, &["input_tokens"])?;
    let cached_input_tokens =
        extract_u64(value, &["cached_input_tokens", "cache_read_input_tokens"]).unwrap_or(0);
    let output_tokens = extract_u64(value, &["output_tokens"]).unwrap_or(0);
    let reasoning_output_tokens = extract_u64(value, &["reasoning_output_tokens"]).unwrap_or(0);
    let total_tokens =
        extract_u64(value, &["total_tokens"]).unwrap_or(input_tokens.saturating_add(output_tokens));

    Some(CodexRawUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
    })
}

pub(super) fn subtract_codex_raw_usage(
    current: CodexRawUsage,
    previous: Option<CodexRawUsage>,
) -> CodexRawUsage {
    let prev = previous.unwrap_or_default();
    CodexRawUsage {
        input_tokens: current.input_tokens.saturating_sub(prev.input_tokens),
        cached_input_tokens: current
            .cached_input_tokens
            .saturating_sub(prev.cached_input_tokens),
        output_tokens: current.output_tokens.saturating_sub(prev.output_tokens),
        reasoning_output_tokens: current
            .reasoning_output_tokens
            .saturating_sub(prev.reasoning_output_tokens),
        total_tokens: current.total_tokens.saturating_sub(prev.total_tokens),
    }
}

pub(super) fn codex_delta_to_usage(delta: CodexRawUsage) -> UsageAccumulator {
    UsageAccumulator {
        input_tokens: delta.input_tokens.saturating_sub(delta.cached_input_tokens),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: delta.cached_input_tokens,
        output_tokens: delta.output_tokens,
        reasoning_output_tokens: delta.reasoning_output_tokens,
        cost_usd: 0.0,
    }
}

pub(super) fn extract_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let candidate_paths = [
        "timestamp",
        "created_at",
        "createdAt",
        "message.timestamp",
        "message.created_at",
    ];

    for path in candidate_paths {
        if let Some(v) = get_value(value, path)
            && let Some(ts) = parse_timestamp_value(v)
        {
            return Some(ts);
        }
    }

    None
}

pub(super) fn parse_timestamp_value(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        Value::Number(n) => {
            let raw = n.as_i64().or_else(|| n.as_u64().map(|u| u as i64))?;
            if raw > 10_000_000_000 {
                DateTime::from_timestamp_millis(raw)
            } else {
                DateTime::from_timestamp(raw, 0)
            }
        }
        _ => None,
    }
}

pub(super) fn extract_u64(value: &Value, paths: &[&str]) -> Option<u64> {
    paths.iter().find_map(|path| {
        let v = get_value(value, path)?;
        match v {
            Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|i| i.max(0) as u64)),
            Value::String(s) => u64::from_str(s).ok(),
            _ => None,
        }
    })
}

pub(super) fn extract_string(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        get_value(value, path)
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    })
}

pub(super) fn get_value<'a>(value: &'a Value, dotted_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in dotted_path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

pub(super) async fn load_pricing(path: Option<&str>, offline: bool) -> Result<PricingTable> {
    let mut table = PricingTable::default_table();

    let (openrouter_exact, litellm_exact) = tokio::join!(
        load_openrouter_pricing_with_cache(!offline),
        load_litellm_pricing_with_cache(!offline),
    );

    if let Some(openrouter_exact) = openrouter_exact {
        table.merge_exact(openrouter_exact);
    }
    if let Some(litellm_exact) = litellm_exact {
        table.merge_exact(litellm_exact);
    }

    if let Some(path) = path {
        let file_path = expand_user_path(path);
        let body = fs::read_to_string(&file_path)
            .await
            .with_context(|| format!("Failed to read pricing file: {}", file_path.display()))?;

        let overrides: HashMap<String, PricingRate> = serde_json::from_str(&body)
            .context("Pricing file must be a JSON object of model -> rate")?;

        table.merge_exact(overrides);
    }

    Ok(table)
}

pub(super) async fn load_openrouter_pricing_with_cache(
    allow_network_fetch: bool,
) -> Option<HashMap<String, PricingRate>> {
    let cache_path = openrouter_pricing_cache_path();
    let cached = cache_path
        .as_ref()
        .and_then(|path| load_openrouter_pricing_cache(path));
    let now = unix_now_secs();

    if let Some(cache) = cached.as_ref()
        && now.saturating_sub(cache.fetched_unix) < OPENROUTER_PRICING_CACHE_TTL_SECS
    {
        return Some(cache.exact.clone());
    }

    if !allow_network_fetch {
        return cached.map(|cache| cache.exact);
    }

    match fetch_openrouter_pricing().await {
        Ok(exact) => {
            if let Some(path) = cache_path.as_ref() {
                save_pricing_cache(path, OPENROUTER_PRICING_CACHE_VERSION, now, &exact);
            }
            Some(exact)
        }
        Err(_) => cached.map(|cache| cache.exact),
    }
}

pub(super) async fn load_litellm_pricing_with_cache(
    allow_network_fetch: bool,
) -> Option<HashMap<String, PricingRate>> {
    let cache_path = litellm_pricing_cache_path();
    let cached = cache_path
        .as_ref()
        .and_then(|path| load_pricing_cache(path, LITELLM_PRICING_CACHE_VERSION));
    let now = unix_now_secs();

    if let Some(cache) = cached.as_ref()
        && now.saturating_sub(cache.fetched_unix) < LITELLM_PRICING_CACHE_TTL_SECS
    {
        return Some(cache.exact.clone());
    }

    if !allow_network_fetch {
        return cached.map(|cache| cache.exact);
    }

    match fetch_litellm_pricing().await {
        Ok(exact) => {
            if let Some(path) = cache_path.as_ref() {
                save_pricing_cache(path, LITELLM_PRICING_CACHE_VERSION, now, &exact);
            }
            Some(exact)
        }
        Err(_) => cached.map(|cache| cache.exact),
    }
}

pub(super) async fn fetch_litellm_pricing() -> Result<HashMap<String, PricingRate>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("tokenusage/0.1")
        .build()
        .context("Failed to initialize LiteLLM pricing client")?;

    let response = client
        .get(LITELLM_PRICING_URL)
        .send()
        .await
        .context("Failed to fetch LiteLLM model pricing")?
        .error_for_status()
        .context("LiteLLM model pricing request failed")?;

    let document: serde_json::Map<String, Value> = response
        .json()
        .await
        .context("Failed to decode LiteLLM model pricing response")?;

    Ok(parse_litellm_rate_table(&document))
}

pub(super) async fn fetch_openrouter_pricing() -> Result<HashMap<String, PricingRate>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("tokenusage/0.1")
        .build()
        .context("Failed to initialize OpenRouter pricing client")?;

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .context("Failed to fetch OpenRouter model pricing")?
        .error_for_status()
        .context("OpenRouter model pricing request failed")?;

    let payload: OpenRouterModelsResponse = response
        .json()
        .await
        .context("Failed to decode OpenRouter model pricing response")?;

    let mut exact = HashMap::new();
    for model in payload.data {
        let Some(rate) = openrouter_rate(&model.pricing) else {
            continue;
        };

        for alias in openrouter_model_aliases(&model.id) {
            exact.insert(alias, rate.clone());
        }
    }

    Ok(exact)
}

pub(super) fn incremental_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("parse-cache-v3.db"))
}

/// Pre-v3 monolithic JSON cache, migrated into SQLite on first run then removed.
fn legacy_json_cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("tokenusage").join("parse-cache-v2.json"))
}

/// Health snapshot of the parse cache for `tu doctor`.
pub(super) struct IncrementalCacheStats {
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: u64,
    pub entries: Option<usize>,
}

/// Inspect the parse cache without mutating it (no schema creation).
pub(super) fn incremental_cache_stats() -> Option<IncrementalCacheStats> {
    let path = incremental_cache_path()?;
    let exists = path.is_file();
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    // Use the same opener as the real cache (mmap + busy_timeout) so stats see
    // consistent counts even if another process holds -wal.
    let entries = if exists {
        open_cache_db_readonly(&path)
            .ok()
            .and_then(|conn| {
                conn.query_row("SELECT COUNT(*) FROM files_v5", [], |r| r.get::<_, i64>(0))
                    .ok()
            })
            .map(|n| n.max(0) as usize)
    } else {
        None
    };
    Some(IncrementalCacheStats {
        path,
        exists,
        size_bytes,
        entries,
    })
}

fn ensure_files_v5_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS files_v5 (
             cache_key TEXT PRIMARY KEY,
             size INTEGER NOT NULL,
             mtime_secs INTEGER NOT NULL,
             mtime_nanos INTEGER NOT NULL,
             parsed_offset INTEGER NOT NULL,
             min_ts_ms INTEGER NOT NULL,
             max_ts_ms INTEGER NOT NULL,
             lines_total INTEGER NOT NULL,
             lines_invalid_json INTEGER NOT NULL,
             lines_missing_usage INTEGER NOT NULL,
             blob BLOB NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_files_v5_ts ON files_v5 (max_ts_ms, min_ts_ms);",
    )
}

fn open_cache_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    // WAL keeps concurrent readers (e.g. statusline alongside an interactive run)
    // from blocking; NORMAL sync is durable enough for a rebuildable cache.
    let _ = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA mmap_size=268435456;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-64000;",
    );
    ensure_files_v5_schema(&conn)?;
    Ok(conn)
}

fn open_cache_db_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let _ = conn.execute_batch(
        "PRAGMA mmap_size=268435456;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-64000;",
    );
    Ok(conn)
}

fn cache_meta_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
}

fn cache_meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

fn reset_stale_cache_db(conn: &Connection, pricing_key: &str) -> rusqlite::Result<()> {
    let _ = conn.execute("DROP TABLE IF EXISTS files", []);
    conn.execute("DELETE FROM files_v5", [])?;
    cache_meta_set(conn, "version", &INCREMENTAL_CACHE_VERSION.to_string())?;
    cache_meta_set(conn, "pricing_key", pricing_key)?;
    // Reclaim space in-place when upgrading from legacy huge JSON-blob caches.
    let _ = conn.execute_batch("VACUUM;");
    Ok(())
}

const V5_UPSERT_SQL: &str = "INSERT OR REPLACE INTO files_v5 (
    cache_key, size, mtime_secs, mtime_nanos, parsed_offset,
    min_ts_ms, max_ts_ms, lines_total, lines_invalid_json, lines_missing_usage, blob
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

fn upsert_v5_row(
    stmt: &mut rusqlite::Statement<'_>,
    key: &str,
    entry: &CachedFileEntry,
) -> rusqlite::Result<()> {
    let (min_ts_ms, max_ts_ms) = entry_ts_bounds(entry);
    let blob = encode_cached_file_entry(entry);
    stmt.execute(params![
        key,
        entry.fingerprint.size as i64,
        entry.fingerprint.modified_unix_secs,
        i64::from(entry.fingerprint.modified_unix_nanos),
        entry.parsed_offset as i64,
        min_ts_ms,
        max_ts_ms,
        entry.stats.lines_total as i64,
        entry.stats.lines_invalid_json as i64,
        entry.stats.lines_missing_usage as i64,
        blob,
    ])?;
    Ok(())
}

/// One-time import of the legacy JSON cache into an empty SQLite DB. The JSON
/// file is removed afterwards (whether imported or stale) so it's never retried.
fn migrate_legacy_json_if_present(conn: &Connection, pricing_key: &str) {
    let Some(json_path) = legacy_json_cache_path() else {
        return;
    };
    if json_path.symlink_metadata().is_err() {
        return;
    }
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM files_v5", [], |r| r.get(0))
        .unwrap_or(0);
    if existing > 0 {
        return;
    }
    let _ = import_legacy_json(conn, pricing_key, &json_path);
    // Remove the legacy file whether imported or stale, so it's never retried.
    let _ = std::fs::remove_file(&json_path);
}

/// Import a legacy JSON cache file into the (empty) SQLite `files_v5` table when its
/// version + pricing key match. Returns Ok(true) when rows were imported.
fn import_legacy_json(
    conn: &Connection,
    pricing_key: &str,
    json_path: &Path,
) -> rusqlite::Result<bool> {
    let Ok(body) = std::fs::read(json_path) else {
        return Ok(false);
    };
    let Ok(store) = serde_json::from_slice::<IncrementalCacheStore>(&body) else {
        return Ok(false);
    };
    if store.version != INCREMENTAL_CACHE_VERSION || store.pricing_key != pricing_key {
        return Ok(false);
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(V5_UPSERT_SQL)?;
        for (key, entry) in &store.files {
            upsert_v5_row(&mut stmt, key, entry)?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('version', ?1)",
        params![INCREMENTAL_CACHE_VERSION.to_string()],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('pricing_key', ?1)",
        params![pricing_key],
    )?;
    tx.commit()?;
    Ok(true)
}

pub(super) fn load_incremental_cache(path: &Path, pricing_key: &str) -> IncrementalCacheStore {
    load_incremental_cache_inner(path, pricing_key)
        .unwrap_or_else(|_| IncrementalCacheStore::new(pricing_key.to_string()))
}

fn load_incremental_cache_inner(
    path: &Path,
    pricing_key: &str,
) -> rusqlite::Result<IncrementalCacheStore> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = open_cache_db(path)?;
    migrate_legacy_json_if_present(&conn, pricing_key);

    let version_ok = cache_meta_get(&conn, "version")?.as_deref()
        == Some(&INCREMENTAL_CACHE_VERSION.to_string());
    let pricing_ok = cache_meta_get(&conn, "pricing_key")?.as_deref() == Some(pricing_key);
    if !version_ok || !pricing_ok {
        reset_stale_cache_db(&conn, pricing_key)?;
        return Ok(IncrementalCacheStore::new(pricing_key.to_string()));
    }

    let mut files = HashMap::new();
    let mut meta = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT cache_key, size, mtime_secs, mtime_nanos, parsed_offset,
                    min_ts_ms, max_ts_ms, lines_total, lines_invalid_json, lines_missing_usage, blob
             FROM files_v5",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let fp = FileFingerprint {
                size: row.get::<_, i64>(1)?.max(0) as u64,
                modified_unix_secs: row.get(2)?,
                modified_unix_nanos: row.get::<_, i64>(3)?.max(0) as u32,
            };
            let parsed_offset = row.get::<_, i64>(4)?.max(0) as u64;
            let min_ts_ms: i64 = row.get(5)?;
            let max_ts_ms: i64 = row.get(6)?;
            let file_stats = CachedFileStats {
                lines_total: row.get::<_, i64>(7)?.max(0) as usize,
                lines_invalid_json: row.get::<_, i64>(8)?.max(0) as usize,
                lines_missing_usage: row.get::<_, i64>(9)?.max(0) as usize,
                lines_unknown_pricing: 0,
            };
            let blob = row.get_ref(10)?.as_bytes()?;
            if let Ok(entry) = decode_cached_file_entry(blob, fp, file_stats.clone(), parsed_offset)
            {
                meta.insert(
                    key.clone(),
                    CachedFileMeta {
                        fingerprint: fp,
                        min_ts_ms,
                        max_ts_ms,
                        parsed_offset,
                        stats: file_stats,
                    },
                );
                files.insert(key, entry);
            }
        }
    }

    Ok(IncrementalCacheStore {
        version: INCREMENTAL_CACHE_VERSION,
        pricing_key: pricing_key.to_string(),
        files,
        meta,
        meta_only: false,
        changed: HashSet::new(),
        removed: HashSet::new(),
        full_rewrite: false,
    })
}

/// Load ONLY per-file metadata and timestamp bounds so date-filtered commands skip
/// decoding or even reading out-of-window blobs from SQLite.
pub(super) fn load_incremental_cache_meta(path: &Path, pricing_key: &str) -> IncrementalCacheStore {
    load_incremental_cache_meta_inner(path, pricing_key).unwrap_or_else(|_| {
        let mut store = IncrementalCacheStore::new(pricing_key.to_string());
        store.meta_only = true;
        store
    })
}

fn load_incremental_cache_meta_inner(
    path: &Path,
    pricing_key: &str,
) -> rusqlite::Result<IncrementalCacheStore> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = open_cache_db(path)?;
    migrate_legacy_json_if_present(&conn, pricing_key);

    let version_ok = cache_meta_get(&conn, "version")?.as_deref()
        == Some(&INCREMENTAL_CACHE_VERSION.to_string());
    let pricing_ok = cache_meta_get(&conn, "pricing_key")?.as_deref() == Some(pricing_key);
    if !version_ok || !pricing_ok {
        reset_stale_cache_db(&conn, pricing_key)?;
        let mut store = IncrementalCacheStore::new(pricing_key.to_string());
        store.meta_only = true;
        return Ok(store);
    }

    let mut meta = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT cache_key, size, mtime_secs, mtime_nanos, parsed_offset,
                    min_ts_ms, max_ts_ms, lines_total, lines_invalid_json, lines_missing_usage
             FROM files_v5",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            meta.insert(
                key,
                CachedFileMeta {
                    fingerprint: FileFingerprint {
                        size: row.get::<_, i64>(1)?.max(0) as u64,
                        modified_unix_secs: row.get(2)?,
                        modified_unix_nanos: row.get::<_, i64>(3)?.max(0) as u32,
                    },
                    min_ts_ms: row.get(5)?,
                    max_ts_ms: row.get(6)?,
                    parsed_offset: row.get::<_, i64>(4)?.max(0) as u64,
                    stats: CachedFileStats {
                        lines_total: row.get::<_, i64>(7)?.max(0) as usize,
                        lines_invalid_json: row.get::<_, i64>(8)?.max(0) as usize,
                        lines_missing_usage: row.get::<_, i64>(9)?.max(0) as usize,
                        lines_unknown_pricing: 0,
                    },
                },
            );
        }
    }

    Ok(IncrementalCacheStore {
        version: INCREMENTAL_CACHE_VERSION,
        pricing_key: pricing_key.to_string(),
        files: HashMap::new(),
        meta,
        meta_only: true,
        changed: HashSet::new(),
        removed: HashSet::new(),
        full_rewrite: false,
    })
}

pub(super) fn save_incremental_cache(path: &Path, store: &mut IncrementalCacheStore) {
    if save_incremental_cache_inner(path, store).is_ok() {
        store.changed.clear();
        store.removed.clear();
        store.full_rewrite = false;
    }
}

fn save_incremental_cache_inner(
    path: &Path,
    store: &IncrementalCacheStore,
) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut conn = open_cache_db(path)?;
    let tx = conn.transaction()?;
    if store.full_rewrite {
        let _ = tx.execute("DROP TABLE IF EXISTS files", []);
        tx.execute("DELETE FROM files_v5", [])?;
        let mut stmt = tx.prepare(V5_UPSERT_SQL)?;
        for (key, entry) in &store.files {
            upsert_v5_row(&mut stmt, key, entry)?;
        }
    } else {
        {
            let mut up = tx.prepare(V5_UPSERT_SQL)?;
            for key in &store.changed {
                if let Some(entry) = store.files.get(key) {
                    upsert_v5_row(&mut up, key, entry)?;
                }
            }
        }
        {
            let mut del = tx.prepare("DELETE FROM files_v5 WHERE cache_key = ?1")?;
            for key in &store.removed {
                del.execute(params![key])?;
            }
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('version', ?1)",
        params![INCREMENTAL_CACHE_VERSION.to_string()],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('pricing_key', ?1)",
        params![store.pricing_key],
    )?;
    tx.commit()
}

pub(super) fn cache_file_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn read_file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(FileFingerprint {
        size: meta.len(),
        modified_unix_secs: dur.as_secs() as i64,
        modified_unix_nanos: dur.subsec_nanos(),
    })
}

pub(super) fn can_incremental_parse(cached: &CachedFileEntry, current: FileFingerprint) -> bool {
    if current.size <= cached.fingerprint.size {
        return false;
    }
    let start_offset = cached.parsed_offset.max(cached.fingerprint.size);
    start_offset > 0 && start_offset <= current.size
}

pub(super) fn can_incremental_parse_meta(meta: &CachedFileMeta, current: FileFingerprint) -> bool {
    if current.size <= meta.fingerprint.size {
        return false;
    }
    let start_offset = meta.parsed_offset.max(meta.fingerprint.size);
    start_offset > 0 && start_offset <= current.size
}

fn date_filter_allows(
    filter: DateFilter,
    timestamp: DateTime<Utc>,
    timezone: &TimeZoneMode,
) -> bool {
    (filter.since.is_none() && filter.until.is_none())
        || filter.allows(local_date(timestamp, timezone))
}

#[inline]
fn date_filter_allows_cached(
    filter: DateFilter,
    timestamp: DateTime<Utc>,
    day_cache: &mut LocalDayCache<'_>,
) -> bool {
    (filter.since.is_none() && filter.until.is_none())
        || filter.allows(day_cache.date_of(timestamp))
}

struct CachedHydrationRateCache<'a> {
    pricing: &'a PricingTable,
    last_model: Option<String>,
    last_rate: Option<&'a PricingRate>,
}

impl<'a> CachedHydrationRateCache<'a> {
    fn new(pricing: &'a PricingTable) -> Self {
        Self {
            pricing,
            last_model: None,
            last_rate: None,
        }
    }

    #[inline]
    fn estimate(
        &mut self,
        model: &str,
        usage: UsageAccumulator,
        ctx: PricingContext,
    ) -> Option<f64> {
        if let Some(reported) = ctx.reported_cost_usd
            && reported.is_finite()
            && reported >= 0.0
        {
            return Some(reported);
        }
        if self.last_model.as_deref() != Some(model) {
            self.last_model = Some(model.to_string());
            self.last_rate = self.pricing.find_rate(model);
        }
        self.pricing
            .estimate_cost_with_rate_and_context(self.last_rate, usage, ctx)
    }
}

fn hydrate_cached_stats_and_cost(cached: &CachedFileEntry, stats: &ParseStatsAtomic) {
    stats
        .lines_total
        .fetch_add(cached.stats.lines_total, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(cached.stats.lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(cached.stats.lines_missing_usage, Ordering::Relaxed);
}

pub(super) fn hydrate_cached_internal_events(
    file: &DiscoveredFile,
    cached: &CachedFileEntry,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> Vec<InternalEvent> {
    hydrate_cached_stats_and_cost(cached, stats);

    let session_meta = std::cell::LazyCell::new(|| derive_session_meta(file));
    let file_path = std::cell::LazyCell::new(|| file.path.display().to_string());
    let mut filtered = 0usize;
    let mut parsed = 0usize;
    let mut unknown_pricing = 0usize;
    let mut events = Vec::with_capacity(cached.events.len());
    let mut rate_cache = CachedHydrationRateCache::new(pricing);
    let mut day_cache = LocalDayCache::new(timezone);

    for cached_event in &cached.events {
        let ctx = cached_event.pricing_context();
        let mut usage = cached_event.usage;
        match rate_cache.estimate(&cached_event.model, usage, ctx) {
            Some(cost) => usage.cost_usd = cost,
            None => {
                usage.cost_usd = 0.0;
                unknown_pricing += 1;
            }
        }

        if !date_filter_allows_cached(filter, cached_event.timestamp, &mut day_cache) {
            filtered += 1;
            continue;
        }
        parsed += 1;
        events.push(InternalEvent {
            event: UsageEvent {
                timestamp: cached_event.timestamp,
                source: file.source,
                model: cached_event.model.clone(),
                session: cached_event
                    .session
                    .clone()
                    .unwrap_or_else(|| session_meta.0.clone()),
                project: cached_event
                    .project
                    .clone()
                    .or_else(|| session_meta.1.clone()),
                file_path: cached_event
                    .file_path
                    .clone()
                    .unwrap_or_else(|| (*file_path).clone()),
                usage,
            },
            pricing_ctx: ctx,
            dedupe_hash: cached_event.dedupe_hash,
        });
    }

    stats.lines_filtered.fetch_add(filtered, Ordering::Relaxed);
    stats.lines_parsed.fetch_add(parsed, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(unknown_pricing, Ordering::Relaxed);

    events
}

fn hydrate_cached_internal_events_owned(
    file: &DiscoveredFile,
    cached: CachedFileEntry,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> Vec<InternalEvent> {
    hydrate_cached_stats_and_cost(&cached, stats);

    let session_meta = std::cell::LazyCell::new(|| derive_session_meta(file));
    let file_path = std::cell::LazyCell::new(|| file.path.display().to_string());
    let mut filtered = 0usize;
    let mut parsed = 0usize;
    let mut unknown_pricing = 0usize;
    let mut events = Vec::with_capacity(cached.events.len());
    let mut rate_cache = CachedHydrationRateCache::new(pricing);
    let mut day_cache = LocalDayCache::new(timezone);

    for cached_event in cached.events {
        let ctx = cached_event.pricing_context();
        let mut usage = cached_event.usage;
        match rate_cache.estimate(&cached_event.model, usage, ctx) {
            Some(cost) => usage.cost_usd = cost,
            None => {
                usage.cost_usd = 0.0;
                unknown_pricing += 1;
            }
        }

        if !date_filter_allows_cached(filter, cached_event.timestamp, &mut day_cache) {
            filtered += 1;
            continue;
        }
        parsed += 1;
        events.push(InternalEvent {
            event: UsageEvent {
                timestamp: cached_event.timestamp,
                source: file.source,
                model: cached_event.model,
                session: cached_event
                    .session
                    .unwrap_or_else(|| session_meta.0.clone()),
                project: cached_event.project.or_else(|| session_meta.1.clone()),
                file_path: cached_event
                    .file_path
                    .unwrap_or_else(|| (*file_path).clone()),
                usage,
            },
            pricing_ctx: ctx,
            dedupe_hash: cached_event.dedupe_hash,
        });
    }

    stats.lines_filtered.fetch_add(filtered, Ordering::Relaxed);
    stats.lines_parsed.fetch_add(parsed, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(unknown_pricing, Ordering::Relaxed);

    events
}

#[cfg(test)]
pub(super) fn hydrate_cached_events(
    file: &DiscoveredFile,
    cached: &CachedFileEntry,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> Vec<UsageEvent> {
    hydrate_cached_internal_events(file, cached, filter, timezone, pricing, stats)
        .into_iter()
        .map(|item| item.event)
        .collect()
}

fn cached_usage_event_for_file(
    event: &UsageEvent,
    ctx: PricingContext,
    dedupe_hash: Option<u64>,
    default_file_path: &str,
) -> CachedUsageEvent {
    let mut usage = event.usage;
    usage.cost_usd = 0.0;
    CachedUsageEvent {
        timestamp: event.timestamp,
        model: event.model.clone(),
        usage,
        session: Some(event.session.clone()),
        project: event.project.clone(),
        file_path: (event.file_path != default_file_path).then(|| event.file_path.clone()),
        cache_creation_1h_tokens: ctx.cache_creation_1h_tokens,
        speed: ctx.speed,
        model_calls: ctx.model_calls.max(1),
        reported_cost_usd: ctx.reported_cost_usd,
        dedupe_hash,
    }
}

/// Cache invalidation key — deliberately **independent of pricing**.
fn cache_schema_key() -> String {
    "estimate-v5.1-claude-fix".to_string()
}

pub(super) fn pricing_cache_key(_pricing: &PricingTable) -> String {
    cache_schema_key()
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

pub(super) fn normalized_discovered_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        normalize_path(path)
    }
}

#[cfg(test)]
mod antigravity_proto_tests {
    use super::*;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if v == 0 {
                break;
            }
        }
    }

    fn encode_tag(field_number: u32, wire_type: u32, out: &mut Vec<u8>) {
        encode_varint(((field_number as u64) << 3) | wire_type as u64, out);
    }

    fn encode_varint_field(field_number: u32, value: u64, out: &mut Vec<u8>) {
        encode_tag(field_number, 0, out);
        encode_varint(value, out);
    }

    fn encode_bytes_field(field_number: u32, payload: &[u8], out: &mut Vec<u8>) {
        encode_tag(field_number, 2, out);
        encode_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    fn encode_string_field(field_number: u32, s: &str, out: &mut Vec<u8>) {
        encode_bytes_field(field_number, s.as_bytes(), out);
    }

    /// Builds a synthetic blob matching the assumed Antigravity generation
    /// record shape documented on `extract_antigravity_gen_event`.
    fn build_gen_blob(model: &str, input: u64, output: u64, cache_read: u64, ts: i64) -> Vec<u8> {
        let mut usage = Vec::new();
        encode_varint_field(2, input, &mut usage);
        encode_varint_field(3, output, &mut usage);
        encode_varint_field(5, cache_read, &mut usage);

        let mut wall_clock = Vec::new();
        encode_varint_field(1, ts as u64, &mut wall_clock);

        let mut timing = Vec::new();
        encode_bytes_field(4, &wall_clock, &mut timing);

        let mut f1 = Vec::new();
        encode_bytes_field(4, &usage, &mut f1);
        encode_bytes_field(9, &timing, &mut f1);
        encode_string_field(19, model, &mut f1);

        let mut top = Vec::new();
        encode_bytes_field(1, &f1, &mut top);
        top
    }

    #[test]
    fn decode_varint_single_byte() {
        assert_eq!(decode_varint(&[0x05], 0), Some((5, 1)));
    }

    #[test]
    fn decode_varint_multi_byte() {
        // 300 = 0b1_0010_1100 -> low 7 bits 0101100 with continuation, then 0000010
        let mut buf = Vec::new();
        encode_varint(300, &mut buf);
        assert_eq!(decode_varint(&buf, 0), Some((300, buf.len())));
    }

    #[test]
    fn decode_varint_truncated_returns_none() {
        // Continuation bit set but no following byte.
        assert_eq!(decode_varint(&[0x80], 0), None);
    }

    #[test]
    fn decode_varint_adversarial_overflow_returns_none() {
        // 10 bytes all with the continuation bit set never terminates within
        // 64 bits and must not panic on the shift.
        let bytes = vec![0xFF; 10];
        assert_eq!(decode_varint(&bytes, 0), None);
    }

    #[test]
    fn parse_proto_fields_roundtrips_varint_and_bytes() {
        let mut buf = Vec::new();
        encode_varint_field(1, 42, &mut buf);
        encode_string_field(2, "hello", &mut buf);

        let fields = parse_proto_fields(&buf);
        assert_eq!(find_proto_varint(&fields, 1), Some(42));
        assert_eq!(
            find_proto_bytes(&fields, 2).and_then(|b| std::str::from_utf8(b).ok()),
            Some("hello")
        );
    }

    #[test]
    fn parse_proto_fields_stops_gracefully_on_truncated_length() {
        // A length-delimited field claiming more bytes than are present.
        let mut buf = Vec::new();
        encode_tag(1, 2, &mut buf);
        encode_varint(1000, &mut buf); // claims 1000 bytes, none follow
        let fields = parse_proto_fields(&buf);
        assert!(fields.is_empty());
    }

    #[test]
    fn extract_antigravity_gen_event_full_record() {
        let blob = build_gen_blob("gemini-3.1-pro-low", 100, 20, 5000, 1_700_000_000);
        let event = extract_antigravity_gen_event(&blob).expect("should parse");
        assert_eq!(event.model, "gemini-3.1-pro-low");
        assert_eq!(event.input_tokens, 100);
        assert_eq!(event.output_tokens, 20);
        assert_eq!(event.cache_read_tokens, 5000);
        assert_eq!(event.timestamp_secs, 1_700_000_000);
    }

    #[test]
    fn extract_antigravity_gen_event_all_zero_tokens_returns_none() {
        let blob = build_gen_blob("gemini-3.6-flash", 0, 0, 0, 1_700_000_000);
        assert!(extract_antigravity_gen_event(&blob).is_none());
    }

    #[test]
    fn extract_antigravity_gen_event_missing_wrapper_returns_none() {
        // No field-1 wrapper at all.
        let mut top = Vec::new();
        encode_varint_field(2, 999, &mut top);
        assert!(extract_antigravity_gen_event(&top).is_none());
    }

    #[test]
    fn extract_antigravity_gen_event_missing_model_falls_back_to_default() {
        let mut usage = Vec::new();
        encode_varint_field(2, 10, &mut usage);

        let mut f1 = Vec::new();
        encode_bytes_field(4, &usage, &mut f1);
        // No field 19 (model) present.

        let mut top = Vec::new();
        encode_bytes_field(1, &f1, &mut top);

        let event = extract_antigravity_gen_event(&top).expect("should parse");
        assert_eq!(event.model, "gemini-3.6-flash");
        assert_eq!(event.input_tokens, 10);
    }

    #[test]
    fn extract_antigravity_gen_event_garbage_blob_returns_none() {
        let garbage = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(extract_antigravity_gen_event(&garbage).is_none());
    }

    #[test]
    fn parse_real_antigravity_db_if_present() {
        if let Some(home) = dirs::home_dir() {
            let path = home.join(
                ".gemini/antigravity-cli/conversations/71e85ab1-103f-438a-8717-0cdd4f47a167.db",
            );
            if path.exists() {
                let job = FileParseJob {
                    file: DiscoveredFile {
                        source: SourceKind::Gemini,
                        root: path.parent().unwrap().to_path_buf(),
                        path: path.clone(),
                    },
                    cache_key: "test".to_string(),
                    fingerprint: FileFingerprint {
                        size: 100,
                        modified_unix_secs: 1,
                        modified_unix_nanos: 0,
                    },
                    strategy: ParseStrategy::Full,
                };
                let stats = ParseStatsAtomic::default();
                let pricing = PricingTable::default();
                let res = parse_antigravity_db_file(
                    job,
                    DateFilter {
                        since: None,
                        until: None,
                    },
                    &TimeZoneMode::Local,
                    &pricing,
                    &stats,
                    true,
                );
                assert!(
                    res.is_some(),
                    "parse_antigravity_db_file should return Some"
                );
                let parsed = res.unwrap();
                eprintln!("Parsed {} events from real DB", parsed.events.len());
                assert!(
                    !parsed.events.is_empty(),
                    "Should parse events from real DB"
                );
            }
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn tmp_db(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tu-cache-test-{}-{tag}.db", std::process::id()))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    fn sample_entry(size: u64) -> CachedFileEntry {
        CachedFileEntry {
            fingerprint: FileFingerprint {
                size,
                modified_unix_secs: 1,
                modified_unix_nanos: 0,
            },
            parsed_offset: size,
            ..CachedFileEntry::default()
        }
    }

    #[test]
    fn hydrate_legacy_codex_entry_uses_file_metadata_and_local_date() {
        let path = tmp_db("codex-metadata").with_extension("jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"cwd":"/work/legacy-project"}}"#,
        )
        .unwrap();
        let file = DiscoveredFile {
            source: SourceKind::Codex,
            root: path.parent().unwrap().to_path_buf(),
            path: path.clone(),
        };
        let mut cached = sample_entry(10);
        cached.stats.lines_total = 2;
        for hour in ["15", "16"] {
            let timestamp = DateTime::parse_from_rfc3339(&format!("2026-10-01T{hour}:00:00Z"))
                .unwrap()
                .to_utc();
            cached.events.push(CachedUsageEvent {
                timestamp,
                model: "unpriced-perf-test-model".to_owned(),
                usage: UsageAccumulator {
                    input_tokens: 12,
                    cost_usd: 0.42,
                    ..UsageAccumulator::default()
                },
                ..CachedUsageEvent::default()
            });
        }

        let day = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let stats = ParseStatsAtomic::default();
        let events = hydrate_cached_events(
            &file,
            &cached,
            DateFilter {
                since: Some(day),
                until: Some(day),
            },
            &TimeZoneMode::Named(chrono_tz::Asia::Singapore),
            &PricingTable::default(),
            &stats,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].session,
            path.file_stem().unwrap().to_str().unwrap()
        );
        assert_eq!(events[0].project.as_deref(), Some("legacy-project"));
        assert_eq!(events[0].file_path, path.display().to_string());
        assert_eq!(events[0].usage.cost_usd, 0.0);
        let stats = stats.snapshot();
        assert_eq!(stats.lines_total, 2);
        assert_eq!(stats.lines_parsed, 1);
        assert_eq!(stats.lines_filtered, 1);
        assert_eq!(stats.lines_unknown_pricing, 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn full_rewrite_round_trips_and_clears_changesets() {
        let path = tmp_db("roundtrip");
        cleanup(&path);
        let mut store = IncrementalCacheStore::new("pk1".to_string());
        store.files.insert("a.jsonl".into(), sample_entry(10));
        store.changed.insert("a.jsonl".into());
        save_incremental_cache(&path, &mut store);
        assert!(store.changed.is_empty(), "changed cleared after save");
        assert!(!store.full_rewrite, "full_rewrite reset after save");

        let loaded = load_incremental_cache(&path, "pk1");
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files.get("a.jsonl").unwrap().fingerprint.size, 10);
        assert!(!loaded.full_rewrite, "loaded store is incremental");
        cleanup(&path);
    }

    #[test]
    fn incremental_upsert_and_delete() {
        let path = tmp_db("incr");
        cleanup(&path);
        let mut store = IncrementalCacheStore::new("pk".to_string());
        store.files.insert("a".into(), sample_entry(1));
        store.files.insert("b".into(), sample_entry(2));
        save_incremental_cache(&path, &mut store);

        let mut store = load_incremental_cache(&path, "pk");
        assert_eq!(store.files.len(), 2);
        store.files.insert("a".into(), sample_entry(99));
        store.changed.insert("a".into());
        store.files.remove("b");
        store.removed.insert("b".into());
        save_incremental_cache(&path, &mut store);

        let loaded = load_incremental_cache(&path, "pk");
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files.get("a").unwrap().fingerprint.size, 99);
        assert!(!loaded.files.contains_key("b"));
        cleanup(&path);
    }

    #[test]
    fn pricing_key_change_invalidates_cache() {
        let path = tmp_db("pricing");
        cleanup(&path);
        let mut store = IncrementalCacheStore::new("old".to_string());
        store.files.insert("a".into(), sample_entry(1));
        save_incremental_cache(&path, &mut store);

        let loaded = load_incremental_cache(&path, "new");
        assert!(loaded.files.is_empty(), "different pricing key => empty");
        assert!(loaded.full_rewrite);
        cleanup(&path);
    }

    #[test]
    fn legacy_json_imports_when_compatible() {
        let path = tmp_db("legacy");
        cleanup(&path);
        let json_path = std::env::temp_dir().join(format!("tu-legacy-{}.json", std::process::id()));
        let mut legacy = IncrementalCacheStore::new("pk".to_string());
        legacy.files.insert("x".into(), sample_entry(5));
        std::fs::write(&json_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let conn = open_cache_db(&path).unwrap();
        assert!(import_legacy_json(&conn, "pk", &json_path).unwrap());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files_v5", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Wrong pricing key must not import.
        let path2 = tmp_db("legacy2");
        cleanup(&path2);
        let conn2 = open_cache_db(&path2).unwrap();
        assert!(!import_legacy_json(&conn2, "different", &json_path).unwrap());

        let _ = std::fs::remove_file(&json_path);
        cleanup(&path);
        cleanup(&path2);
    }

    #[test]
    fn parse_grok_session_update_turn_completed() {
        let pricing = PricingTable::default();
        let sample = r#"{"timestamp":1785179745,"method":"_x.ai/session/update","params":{"sessionId":"019fa4ff-8769-76c1-b662-ab0d9141d183","update":{"sessionUpdate":"turn_completed","prompt_id":"019fa4ff-8a6a-7b71-a71e-e681dee271c4","stop_reason":"end_turn","usage":{"inputTokens":659466,"outputTokens":11143,"totalTokens":670609,"cachedReadTokens":608640,"reasoningTokens":6744,"modelCalls":15,"apiDurationMs":102689,"costUsdTicks":1948400000,"modelUsage":{"grok-4.6-build":{"inputTokens":659466,"outputTokens":11143,"totalTokens":670609,"cachedReadTokens":608640,"reasoningTokens":6744,"modelCalls":15,"apiDurationMs":102689,"costUsdTicks":1948400000}},"numTurns":15}},"_meta":{"eventId":"019fa4ff-8769-76c1-b662-ab0d9141d183-38799","agentTimestampMs":1785179745037}}}"#;

        let result = parse_grok_usage_line_records(sample, &pricing);
        let MultiLineResult::Parsed(records) = result else {
            panic!("Expected parsed result");
        };
        let parsed = &records[0];

        assert_eq!(parsed.event.source, SourceKind::Grok);
        assert_eq!(parsed.event.model, "grok-4.6-build");
        assert_eq!(parsed.event.session, "019fa4ff-8769-76c1-b662-ab0d9141d183");
        assert_eq!(parsed.event.usage.input_tokens, 659466 - 608640);
        assert_eq!(parsed.event.usage.cache_read_input_tokens, 608640);
        assert_eq!(parsed.event.usage.output_tokens, 11143);
        assert_eq!(parsed.event.usage.reasoning_output_tokens, 6744);
        assert!((parsed.event.usage.cost_usd - 0.19484).abs() < 1e-6);
        assert_eq!(parsed.pricing_ctx.reported_cost_usd, Some(0.19484));
        assert_eq!(parsed.pricing_ctx.model_calls, 15);
    }

    #[test]
    fn parse_grok_unified_log_turn_inference_done() {
        let pricing = PricingTable::default();
        let sample = r#"{"ts":"2026-08-25T14:12:31.661Z","src":"shell","pid":49284,"ver":"1.0.5","lvl":"info","sid":"01a03943-f501-70f3-a587-28e468a24112","msg":"shell.turn.inference_done","ctx":{"prompt_tokens":1000,"cached_prompt_tokens":800,"completion_tokens":200,"reasoning_tokens":50}}"#;

        let result = parse_grok_usage_line_records(sample, &pricing);
        let MultiLineResult::Parsed(records) = result else {
            panic!("Expected parsed result");
        };
        let parsed = &records[0];

        assert_eq!(parsed.event.source, SourceKind::Grok);
        assert_eq!(parsed.event.session, "01a03943-f501-70f3-a587-28e468a24112");
        assert_eq!(parsed.event.usage.input_tokens, 200);
        assert_eq!(parsed.event.usage.cache_read_input_tokens, 800);
        assert_eq!(parsed.event.usage.output_tokens, 200);
        assert_eq!(parsed.event.usage.reasoning_output_tokens, 50);
    }

    #[test]
    fn decode_grok_project_dir_url_decodes_cleanly() {
        assert_eq!(
            decode_grok_project_dir("%2FUsers%2Ffelix%2FProjects%2Fequity-signals"),
            "equity-signals"
        );
        assert_eq!(decode_grok_project_dir("%2Fprivate%2Ftmp"), "tmp");
        assert_eq!(
            decode_grok_project_dir("standalone-proj"),
            "standalone-proj"
        );
    }

    #[test]
    fn should_not_skip_claude_assistant_after_2048_bytes() {
        let padding = "x".repeat(4096);
        let line = format!(
            r#"{{"padding":"{padding}","type":"assistant","timestamp":"2026-10-01T10:00:00Z","sessionId":"s1","message":{{"id":"msg_1","model":"claude-sonnet-4-6","usage":{{"input_tokens":100,"output_tokens":25}}}}}}"#
        );
        assert!(!should_skip_parse_by_line_prefix(SourceKind::Claude, &line));
        let MultiLineResult::Parsed(records) = parse_claude_usage_line_records(&line) else {
            panic!("expected late assistant line to parse");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event.usage.input_tokens, 100);
        assert_eq!(records[0].event.usage.output_tokens, 25);
    }

    #[test]
    fn claude_max_merge_dedupe_1h_fast_advisor_and_synthetic() {
        let synth = r#"{"type":"assistant","timestamp":"2026-10-01T10:00:00Z","message":{"id":"m0","model":"<synthetic>","usage":{"input_tokens":50,"output_tokens":5}}}"#;
        assert!(matches!(
            parse_claude_usage_line_records(synth),
            MultiLineResult::MissingUsage
        ));

        let row1 = r#"{"type":"assistant","timestamp":"2026-10-01T10:00:01Z","sessionId":"s1","requestId":"r1","message":{"id":"m1","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_creation":{"ephemeral_1h_input_tokens":100},"cache_read_input_tokens":50,"output_tokens":5,"speed":"fast"}}}"#;
        let row2 = r#"{"type":"assistant","timestamp":"2026-10-01T10:00:02Z","sessionId":"s1","requestId":"r1","message":{"id":"m1","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_creation":{"ephemeral_1h_input_tokens":100},"cache_read_input_tokens":50,"output_tokens":42,"speed":"fast","iterations":[{"type":"advisor_message","model":"claude-opus-4-6","input_tokens":50,"output_tokens":20}]}}}"#;

        let MultiLineResult::Parsed(r2) = parse_claude_usage_line_records(row2) else {
            panic!("expected row2 to parse");
        };
        assert_eq!(r2.len(), 2);
        assert_eq!(r2[0].pricing_ctx.cache_creation_1h_tokens, 100);
        assert_eq!(r2[0].pricing_ctx.speed, ServiceSpeed::Fast);
        assert_eq!(r2[0].event.usage.input_tokens, 10);
        assert_eq!(r2[0].event.usage.output_tokens, 42);
        assert_eq!(r2[1].event.model, "claude-opus-4-6");
        assert_eq!(r2[1].event.usage.input_tokens, 50);
        assert_eq!(r2[1].event.usage.output_tokens, 20);

        let dir = std::env::temp_dir().join(format!("tu-claude-dedupe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f1 = dir.join("a.jsonl");
        let f2 = dir.join("b.jsonl");
        std::fs::write(&f1, format!("{row1}\n")).unwrap();
        std::fs::write(&f2, format!("{row2}\n")).unwrap();

        let files = vec![
            DiscoveredFile {
                source: SourceKind::Claude,
                root: dir.clone(),
                path: f1,
            },
            DiscoveredFile {
                source: SourceKind::Claude,
                root: dir.clone(),
                path: f2,
            },
        ];
        let mut store = IncrementalCacheStore::new("pk".into());
        let out = parse_files_with_cache(
            &files,
            &mut store,
            ParseFilesConfig {
                filter: DateFilter::default(),
                timezone: &TimeZoneMode::Utc,
                pricing: Arc::new(PricingTable::default()),
                worker_count: 1,
                cache_enabled: false,
                cache_path: None,
                sort_events: true,
            },
        );
        assert_eq!(out.loaded.events.len(), 2);
        let main_ev = out
            .loaded
            .events
            .iter()
            .find(|e| e.model == "claude-sonnet-4-6")
            .unwrap();
        let adv_ev = out
            .loaded
            .events
            .iter()
            .find(|e| e.model == "claude-opus-4-6")
            .unwrap();
        assert_eq!(main_ev.usage.input_tokens, 10);
        assert_eq!(main_ev.usage.output_tokens, 42);
        assert_eq!(adv_ev.usage.input_tokens, 50);
        assert_eq!(adv_ev.usage.output_tokens, 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grok_multi_model_split_cache_creation_and_cliff_guard() {
        let pricing = PricingTable::default_table();
        // 500K total input across 10 calls = 50K/call <= 200K cliff => standard rate ($2/M, $6/M), not cliff rate.
        let sample = r#"{"timestamp":1785179745,"method":"_x.ai/session/update","params":{"sessionId":"s-multi","update":{"sessionUpdate":"turn_completed","prompt_id":"p1","usage":{"inputTokens":500000,"outputTokens":1000,"costUsdTicks":0,"modelCalls":10,"modelUsage":{"grok-4.6":{"inputTokens":500000,"outputTokens":1000,"cacheCreationTokens":4000,"cachedReadTokens":0,"modelCalls":10,"costUsdTicks":0},"grok-4.6-fast":{"inputTokens":1000,"outputTokens":500,"cachedReadTokens":0,"modelCalls":1,"costUsdTicks":15000000}}}},"_meta":{"agentTimestampMs":1785179745000}}}"#;

        let MultiLineResult::Parsed(records) = parse_grok_usage_line_records(sample, &pricing)
        else {
            panic!("expected Grok records");
        };
        assert_eq!(records.len(), 2);

        let main = records
            .iter()
            .find(|r| r.event.model == "grok-4.6")
            .expect("grok-4.6 record");
        assert_eq!(main.event.usage.cache_creation_input_tokens, 4000);
        assert_eq!(main.pricing_ctx.model_calls, 10);
        // Reported cost was absent/0, so estimate runs with per-call prompt 50K (< 200K cliff):
        // 0.5M * $2/M + 1000 * $6/M = $1.006 (well below >200K cliff ~$2.012)
        assert!(
            main.event.usage.cost_usd > 0.9 && main.event.usage.cost_usd < 1.5,
            "multi-call turn should use standard (non-cliff) rate: {}",
            main.event.usage.cost_usd
        );

        let fast = records
            .iter()
            .find(|r| r.event.model == "grok-4.6-fast")
            .expect("grok-4.6-fast record");
        assert!((fast.event.usage.cost_usd - 0.0015).abs() < 1e-8);
        assert_eq!(fast.pricing_ctx.reported_cost_usd, Some(0.0015));

        // Ensure TU05 codec + hydrate_cached_internal_events preserves context & reported cost.
        let mut entry = sample_entry(128);
        for rec in &records {
            entry.events.push(cached_usage_event_for_file(
                &rec.event,
                rec.pricing_ctx,
                rec.dedupe_hash,
                "x",
            ));
        }
        let blob = encode_cached_file_entry(&entry);
        let decoded = decode_cached_file_entry(&blob, entry.fingerprint, entry.stats, 128).unwrap();
        let file = DiscoveredFile {
            source: SourceKind::Grok,
            root: PathBuf::from("/tmp"),
            path: PathBuf::from("/tmp/updates.jsonl"),
        };
        let stats = ParseStatsAtomic::default();
        let hydrated = hydrate_cached_internal_events(
            &file,
            &decoded,
            DateFilter::default(),
            &TimeZoneMode::Utc,
            &pricing,
            &stats,
        );
        assert_eq!(hydrated.len(), 2);
        let hydrated_fast = hydrated
            .iter()
            .find(|e| e.event.model == "grok-4.6-fast")
            .unwrap();
        assert!((hydrated_fast.event.usage.cost_usd - 0.0015).abs() < 1e-8);
    }

    #[test]
    fn codex_duplicate_token_count_fork_and_service_tier() {
        let pricing = PricingTable::default_table();
        let mut state = CodexParseState::default();

        let fork_meta = r#"{"timestamp":"2026-10-01T10:00:00Z","type":"session_meta","payload":{"forked_from_id":"parent-123"}}"#;
        assert!(matches!(
            parse_codex_usage_line_records(fork_meta, &mut state, &pricing),
            MultiLineResult::MissingUsage
        ));
        assert!(state.fork_anchor_ts.is_some());

        let turn_ctx = r#"{"timestamp":"2026-10-01T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.4","cwd":"/work/app"}}"#;
        assert!(matches!(
            parse_codex_usage_line_records(turn_ctx, &mut state, &pricing),
            MultiLineResult::MissingUsage
        ));

        let tier_line = r#"{"timestamp":"2026-10-01T10:00:02Z","type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"service_tier":"fast"}}}"#;
        assert!(matches!(
            parse_codex_usage_line_records(tier_line, &mut state, &pricing),
            MultiLineResult::MissingUsage
        ));
        assert_eq!(state.service_tier, ServiceSpeed::Fast);

        let tok_line = r#"{"timestamp":"2026-10-01T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":0},"total_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":0,"total_tokens":1100}}}}"#;
        let MultiLineResult::Parsed(first) =
            parse_codex_usage_line_records(tok_line, &mut state, &pricing)
        else {
            panic!("expected first token_count to parse");
        };
        assert_eq!(first[0].pricing_ctx.speed, ServiceSpeed::Fast);
        // Fast doubles gpt-5.4 price ($2.5/M input + $15/M output) * 2
        let expected_std = 1000.0 * 2.5e-6 + 100.0 * 15e-6;
        assert!((first[0].event.usage.cost_usd - expected_std * 2.0).abs() < 1e-8);

        // Identical duplicate token_count emission must be ignored.
        assert!(matches!(
            parse_codex_usage_line_records(tok_line, &mut state, &pricing),
            MultiLineResult::MissingUsage
        ));
    }

    #[test]
    fn tu05_codec_and_meta_date_pushdown() {
        let path = tmp_db("meta-pushdown");
        cleanup(&path);
        let dir = std::env::temp_dir().join(format!("tu-pushdown-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let old_path = dir.join("old.jsonl");
        let new_path = dir.join("new.jsonl");
        std::fs::write(&old_path, "{}\n").unwrap();
        std::fs::write(&new_path, "{}\n").unwrap();

        let old_fp = read_file_fingerprint(&old_path).unwrap();
        let new_fp = read_file_fingerprint(&new_path).unwrap();

        let mut old_entry = sample_entry(old_fp.size);
        old_entry.fingerprint = old_fp;
        old_entry.codex_service_tier = ServiceSpeed::Ultrafast;
        old_entry.codex_fork_anchor_ms = Some(123_456);
        old_entry.codex_request_ledger_active = true;
        old_entry.events.push(CachedUsageEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-01-10T12:00:00Z")
                .unwrap()
                .to_utc(),
            model: "claude-sonnet-4-6".to_string(),
            usage: UsageAccumulator {
                input_tokens: 100,
                output_tokens: 10,
                ..UsageAccumulator::default()
            },
            cache_creation_1h_tokens: 55,
            speed: ServiceSpeed::Fast,
            model_calls: 3,
            reported_cost_usd: Some(0.1234),
            dedupe_hash: Some(42),
            ..CachedUsageEvent::default()
        });

        let decoded = decode_cached_file_entry(
            &encode_cached_file_entry(&old_entry),
            old_fp,
            old_entry.stats.clone(),
            old_entry.parsed_offset,
        )
        .unwrap();
        assert_eq!(decoded.codex_service_tier, ServiceSpeed::Ultrafast);
        assert_eq!(decoded.codex_fork_anchor_ms, Some(123_456));
        assert!(decoded.codex_request_ledger_active);
        assert_eq!(decoded.events[0].cache_creation_1h_tokens, 55);
        assert_eq!(decoded.events[0].speed, ServiceSpeed::Fast);
        assert_eq!(decoded.events[0].model_calls, 3);
        assert_eq!(decoded.events[0].reported_cost_usd, Some(0.1234));
        assert_eq!(decoded.events[0].dedupe_hash, Some(42));

        let mut new_entry = sample_entry(new_fp.size);
        new_entry.fingerprint = new_fp;
        new_entry.events.push(CachedUsageEvent {
            timestamp: DateTime::parse_from_rfc3339("2026-11-20T12:00:00Z")
                .unwrap()
                .to_utc(),
            model: "claude-sonnet-4-6".to_string(),
            usage: UsageAccumulator {
                input_tokens: 200,
                output_tokens: 20,
                ..UsageAccumulator::default()
            },
            ..CachedUsageEvent::default()
        });

        let old_key = cache_file_key(&old_path);
        let new_key = cache_file_key(&new_path);
        let mut store = IncrementalCacheStore::new("pk-meta".into());
        store.files.insert(old_key.clone(), old_entry);
        store.files.insert(new_key, new_entry);
        save_incremental_cache(&path, &mut store);

        // Corrupt the old blob in SQLite: if meta pushdown wrongly reads old blobs, this would fail.
        {
            let conn = open_cache_db(&path).unwrap();
            conn.execute(
                "UPDATE files_v5 SET blob = X'DEADBEEF' WHERE cache_key = ?1",
                params![&old_key],
            )
            .unwrap();
        }

        let mut meta_store = load_incremental_cache_meta(&path, "pk-meta");
        assert!(meta_store.meta_only);
        assert!(meta_store.files.is_empty());
        assert_eq!(meta_store.meta.len(), 2);

        let files = vec![
            DiscoveredFile {
                source: SourceKind::Claude,
                root: dir.clone(),
                path: old_path,
            },
            DiscoveredFile {
                source: SourceKind::Claude,
                root: dir.clone(),
                path: new_path,
            },
        ];
        let out = parse_files_with_cache(
            &files,
            &mut meta_store,
            ParseFilesConfig {
                filter: DateFilter {
                    since: NaiveDate::from_ymd_opt(2026, 11, 19),
                    until: NaiveDate::from_ymd_opt(2026, 11, 21),
                },
                timezone: &TimeZoneMode::Utc,
                pricing: Arc::new(PricingTable::default()),
                worker_count: 1,
                cache_enabled: true,
                cache_path: Some(&path),
                sort_events: true,
            },
        );
        assert_eq!(out.loaded.events.len(), 1);
        assert_eq!(out.loaded.events[0].usage.input_tokens, 200);

        let _ = std::fs::remove_dir_all(&dir);
        cleanup(&path);
    }

    #[test]
    fn claude_handles_duplicate_session_id_and_null_iterations() {
        let line = r#"{"type":"assistant","timestamp":"2026-10-01T10:00:00Z","sessionId":"sess-camel","session_id":"sess-snake","requestId":"req-1","request_id":"req-1","message":{"id":"m-dup","model":"claude-opus-5-5","usage":{"input_tokens":120,"cache_creation_input_tokens":500,"cache_read_input_tokens":4000,"output_tokens":80,"iterations":null,"speed":null,"inference_geo":null}}}"#;
        let MultiLineResult::Parsed(records) = parse_claude_usage_line_records(line) else {
            panic!("expected Claude line with duplicate session_id and null iterations to parse");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event.session, "sess-camel");
        assert_eq!(records[0].event.usage.input_tokens, 120);
        assert_eq!(records[0].event.usage.cache_creation_input_tokens, 500);
        assert_eq!(records[0].event.usage.cache_read_input_tokens, 4000);
        assert_eq!(records[0].event.usage.output_tokens, 80);
    }

    #[test]
    fn pi_parses_session_assistant_toolresult_summary_and_reported_cost() {
        let pricing = PricingTable::default_table();
        let mut session_id = String::new();

        let sess = r#"{"type":"session","version":3,"id":"sess-pi-1","timestamp":"2026-10-01T10:00:00Z","cwd":"/Users/dev/app"}"#;
        assert!(matches!(
            parse_pi_usage_line_records(sess, &mut session_id, &pricing),
            MultiLineResult::MissingUsage
        ));
        assert_eq!(session_id, "sess-pi-1");

        let assistant = r#"{"type":"message","id":"msg-1","timestamp":"2026-10-01T10:00:05Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-6","responseModel":"claude-sonnet-4-6","usage":{"input":1000,"output":200,"cacheRead":5000,"cacheWrite":300,"reasoning":50,"cost":{"total":0.125}}}}"#;
        let MultiLineResult::Parsed(recs) =
            parse_pi_usage_line_records(assistant, &mut session_id, &pricing)
        else {
            panic!("expected assistant Pi record");
        };
        assert_eq!(recs[0].event.source, SourceKind::Pi);
        assert_eq!(recs[0].event.session, "sess-pi-1");
        assert_eq!(recs[0].event.model, "anthropic/claude-sonnet-4-6");
        assert_eq!(recs[0].event.usage.input_tokens, 1000);
        assert_eq!(recs[0].event.usage.cache_read_input_tokens, 5000);
        assert_eq!(recs[0].event.usage.cache_creation_input_tokens, 300);
        assert_eq!(recs[0].event.usage.output_tokens, 200);
        assert_eq!(recs[0].event.usage.reasoning_output_tokens, 50);
        assert!((recs[0].event.usage.cost_usd - 0.125).abs() < 1e-8);

        let tool_res = r#"{"type":"message","id":"msg-2","timestamp":"2026-10-01T10:00:06Z","message":{"role":"toolResult","usage":{"input":0,"output":40,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.0}}}}"#;
        let MultiLineResult::Parsed(tool_recs) =
            parse_pi_usage_line_records(tool_res, &mut session_id, &pricing)
        else {
            panic!("expected toolResult Pi record");
        };
        assert_eq!(tool_recs[0].event.model, "Tools/summaries");
        assert_eq!(tool_recs[0].event.usage.output_tokens, 40);
        assert_eq!(tool_recs[0].pricing_ctx.reported_cost_usd, None);

        let compaction = r#"{"type":"compaction","id":"cmp-1","timestamp":"2026-10-01T10:00:10Z","usage":{"input":2000,"output":150,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.02}}}"#;
        let MultiLineResult::Parsed(cmp_recs) =
            parse_pi_usage_line_records(compaction, &mut session_id, &pricing)
        else {
            panic!("expected compaction Pi record");
        };
        assert_eq!(cmp_recs[0].event.model, "Tools/summaries");
        assert!((cmp_recs[0].event.usage.cost_usd - 0.02).abs() < 1e-8);
    }

    #[test]
    fn muse_parses_v1_v3_models_and_dedupe_hash() {
        let pricing = PricingTable::default_table();
        let line_v1 = r#"{"schema_version":1,"record_type":"event","payload_type":"runtime.session","payload_schema_version":1,"id":"ev-1","recorded_at":1760000000123000,"stream":{"kind":"session","id":"sess-muse-1"},"payload":{"source_run_record_id":"run-1","event":{"kind":"model_completed","model":"claude-sonnet-4-6","usage":{"input_tokens":1000,"output_tokens":200,"cache_read_tokens":600,"cache_write_tokens":100,"reasoning_tokens":50}}}}"#;
        let MultiLineResult::Parsed(recs) = parse_muse_usage_line_records(line_v1, &pricing) else {
            panic!("expected Muse v1 record");
        };
        assert_eq!(recs[0].event.source, SourceKind::Muse);
        assert_eq!(recs[0].event.session, "sess-muse-1");
        assert_eq!(recs[0].event.model, "claude-sonnet-4-6");
        assert_eq!(
            recs[0].event.timestamp.timestamp_millis(),
            1_760_000_000_123
        );
        assert_eq!(recs[0].event.usage.input_tokens, 300);
        assert_eq!(recs[0].event.usage.cache_read_input_tokens, 600);
        assert_eq!(recs[0].event.usage.cache_creation_input_tokens, 100);
        assert_eq!(recs[0].event.usage.output_tokens, 200);
        assert_eq!(recs[0].event.usage.reasoning_output_tokens, 50);

        let line_v3 = r#"{"schema_version":1,"record_type":"event","payload_type":"runtime.session","payload_schema_version":3,"id":"ev-2","recorded_at":1760000005000000,"stream":{"kind":"session","id":"sess-muse-2"},"payload":{"event":{"kind":"automated_review_completed","model":{"model_id":"gpt-5.4"},"usage":{"input_tokens":500,"output_tokens":50,"cached_input_tokens":200}}}}"#;
        let MultiLineResult::Parsed(recs_v3) = parse_muse_usage_line_records(line_v3, &pricing)
        else {
            panic!("expected Muse v3 record");
        };
        assert_eq!(recs_v3[0].event.model, "gpt-5.4");
        assert_eq!(recs_v3[0].event.usage.input_tokens, 300);
        assert_eq!(recs_v3[0].event.usage.cache_read_input_tokens, 200);
    }
}

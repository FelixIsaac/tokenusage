use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, TimeZone, Utc};
use crossbeam_channel::{Receiver, bounded};
use ignore::WalkBuilder;
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use tokio::fs;

use crate::cli::CommonArgs;
use crate::types::{
    CodexParseState, CodexRawUsage, DateFilter, DiscoveredFile, LEGACY_CODEX_FALLBACK_MODEL,
    ParseLineResult, ParseStatsAtomic, ParsedLine, PricingRate, PricingTable, SourceConfig,
    SourceKind, UsageAccumulator, UsageEvent,
};

use super::pricing::*;
use super::*;

pub(super) struct ParseFilesConfig<'a> {
    pub filter: DateFilter,
    pub timezone: &'a TimeZoneMode,
    pub pricing: Arc<PricingTable>,
    pub worker_count: usize,
    pub cache_enabled: bool,
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
        sort_events,
    } = config;
    let stats = Arc::new(ParseStatsAtomic::default());
    stats.files_discovered.store(files.len(), Ordering::Relaxed);

    let mut cache_dirty = false;
    let seen_cache_keys: HashSet<String> = files
        .iter()
        .map(|file| cache_file_key(&file.path))
        .collect();

    // The fingerprint(stat) + cache-hydration pass is the per-file floor over all
    // ~N discovered files. It only reads the cache (immutable) and bumps atomic
    // stats, so it parallelizes cleanly across cores. Each file resolves to either
    // already-cached events or a parse job; partition afterwards.
    enum PassResult {
        Hit(Vec<UsageEvent>),
        Job(Box<FileParseJob>),
    }

    let cache_files = &cache_store.files;
    let pass: Vec<PassResult> = files
        .par_iter()
        .map(|file| {
            let key = cache_file_key(&file.path);
            let Some(fingerprint) = read_file_fingerprint(&file.path) else {
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

            if cache_enabled && let Some(cached) = cache_files.get(&key) {
                if cached.fingerprint == fingerprint {
                    return PassResult::Hit(hydrate_cached_events(
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

            PassResult::Job(Box::new(FileParseJob {
                file: file.clone(),
                cache_key: key,
                fingerprint,
                strategy: ParseStrategy::Full,
            }))
        })
        .collect();

    let mut parse_jobs = Vec::new();
    let mut events = Vec::new();
    for result in pass {
        match result {
            PassResult::Hit(hit) => events.extend(hit),
            PassResult::Job(job) => parse_jobs.push(*job),
        }
    }

    let parsed = parse_files_concurrently(
        parse_jobs,
        worker_count.max(1),
        filter,
        timezone.clone(),
        pricing,
        stats.clone(),
    );
    events.extend(parsed.events);

    if cache_enabled {
        for (key, entry) in parsed.cache_updates {
            cache_store.changed.insert(key.clone());
            cache_store.files.insert(key, entry);
            cache_dirty = true;
        }

        // Evict entries for files no longer present — but ONLY under roots this
        // run actually scanned. A scoped (single-source) run, e.g. `tu codex ...`
        // or doctor's opencode-only probe, must not wipe other sources' entries
        // (which it never discovered). Record removals so the backing store can
        // delete exactly those rows (no full rewrite).
        let scanned_root_keys: HashSet<String> = files
            .iter()
            .map(|file| cache_file_key(&file.root))
            .collect();
        let removed_keys: Vec<String> = cache_store
            .files
            .keys()
            .filter(|key| {
                !seen_cache_keys.contains(key.as_str())
                    && scanned_root_keys
                        .iter()
                        .any(|root| key.starts_with(root.as_str()))
            })
            .cloned()
            .collect();
        for key in removed_keys {
            cache_store.files.remove(&key);
            cache_store.changed.remove(&key);
            cache_store.removed.insert(key);
            cache_dirty = true;
        }
    }

    if sort_events {
        events.sort_by_key(|e| e.timestamp);
    }
    dedupe_opencode_events(&mut events);

    ParsedUsageOutput {
        loaded: LoadedUsage {
            events,
            stats: stats.snapshot(),
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

pub(super) async fn load_usage(
    common: &CommonArgs,
    timezone: &TimeZoneMode,
) -> Result<LoadedUsage> {
    let filter = parse_common_filter(common)?;
    let sources = build_sources(common).await?;
    if sources.is_empty() {
        bail!(
            "No valid source directories found. Please provide --claude-projects-dir/--codex-sessions-dir."
        );
    }

    let pricing = Arc::new(load_pricing(common.pricing_file.as_deref(), common.offline).await?);
    let ignore_rules = PathIgnoreRules::from_common(common);
    let files = discover_files(&sources, &ignore_rules, filter);
    let worker_count = worker_count_from_common(common);
    let pricing_key = pricing_cache_key(&pricing);
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

    let parsed = parse_files_with_cache(
        &files,
        &mut cache_store,
        ParseFilesConfig {
            filter,
            timezone,
            pricing,
            worker_count,
            cache_enabled,
            sort_events: true,
        },
    );
    if cache_enabled
        && (parsed.cache_dirty || common.rebuild_cache)
        && let Some(path) = cache_path.as_ref()
    {
        save_incremental_cache(path, &mut cache_store);
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

fn provider_registry() -> [ProviderSpec; 5] {
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
            base.join("antigravity-cli").join("brain"),
            base.join("antigravity-cli"),
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

pub(super) fn discover_files(
    sources: &[SourceConfig],
    ignore_rules: &PathIgnoreRules,
    filter: DateFilter,
) -> Vec<DiscoveredFile> {
    let mut files: Vec<DiscoveredFile> = sources
        .par_iter()
        .flat_map_iter(|source| {
            source.roots.iter().flat_map(move |root| {
                discover_files_in_root(source.kind, root, ignore_rules, filter)
            })
        })
        .collect();

    files.par_sort_unstable_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    files
}

pub(super) fn discover_files_in_root(
    kind: SourceKind,
    root: &Path,
    ignore_rules: &PathIgnoreRules,
    _filter: DateFilter,
) -> Vec<DiscoveredFile> {
    // NOTE:
    // Do not short-circuit Codex discovery by directory date partition.
    // A Codex session file can continue receiving events on later days while
    // staying under its original directory (session start date), so partition
    // pruning can miss valid events and undercount filtered ranges.
    let mut out = Vec::new();

    if kind == SourceKind::OpenCode {
        // Always merge both SQLite and legacy message logs when present.
        let db_path = root.join("opencode.db");
        if db_path.is_file() && !ignore_rules.should_skip_path(&db_path) {
            out.push(DiscoveredFile {
                source: kind,
                root: root.to_path_buf(),
                path: normalized_discovered_path(&db_path),
            });
        }
    }
    let rules = ignore_rules.clone();
    let mut builder = WalkBuilder::new(root);
    builder
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
        .filter_entry(move |entry| entry.depth() == 0 || !rules.should_skip_dir(entry.path()));

    for entry in builder.build().filter_map(Result::ok) {
        let path = entry.path();
        let is_file = entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
        if !is_file || ignore_rules.should_skip_path(path) {
            continue;
        }

        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let should_keep = provider_accepts_extension(kind, ext);
        if !should_keep {
            continue;
        }

        if kind == SourceKind::OpenCode {
            // Restrict OpenCode discovery to message JSONs; skip db/log/snapshots.
            let normalized = path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !normalized.contains("/storage/message/") {
                continue;
            }
        }

        if kind == SourceKind::Grok {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name != "updates.jsonl" && file_name != "unified.jsonl" {
                continue;
            }
        }

        out.push(DiscoveredFile {
            source: kind,
            root: root.to_path_buf(),
            path: normalized_discovered_path(path),
        });
    }

    out
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

pub(super) fn parse_files_concurrently(
    files: Vec<FileParseJob>,
    workers: usize,
    filter: DateFilter,
    timezone: TimeZoneMode,
    pricing: Arc<PricingTable>,
    stats: Arc<ParseStatsAtomic>,
) -> WorkerParseOutput {
    let (tx, rx) = bounded::<FileParseJob>(4096);
    let global_claude_dedupe = Arc::new(ClaudeGlobalDedupe::default());

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
        let global_claude_dedupe = global_claude_dedupe.clone();

        let handle = thread::spawn(move || {
            worker_loop(
                rx,
                filter,
                &timezone,
                &pricing,
                &stats,
                &global_claude_dedupe,
            )
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
    global_claude_dedupe: &ClaudeGlobalDedupe,
) -> WorkerParseOutput {
    let mut output = WorkerParseOutput::default();
    while let Ok(job) = rx.recv() {
        let cache_key = job.cache_key.clone();
        if let Some(parsed) =
            parse_single_file(job, filter, timezone, pricing, stats, global_claude_dedupe)
        {
            output.events.extend(parsed.events);
            output.cache_updates.push((cache_key, parsed.cache_entry));
        }
    }
    output
}

#[derive(Debug, Default)]
pub(super) struct ClaudeGlobalDedupe {
    seen_keys: Mutex<HashSet<String>>,
}

impl ClaudeGlobalDedupe {
    fn insert(&self, key: &str) -> bool {
        let mut seen = self
            .seen_keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.insert(key.to_string())
    }
}

pub(super) fn parse_single_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
    global_claude_dedupe: &ClaudeGlobalDedupe,
) -> Option<ParsedFileOutput> {
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
    let mut claude_state = ClaudeDedupeState::default();

    let mut local_events = Vec::new();
    let mut cached_events = Vec::new();
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
            local_events.extend(hydrate_cached_events(
                &job.file, base_cache, filter, timezone, pricing, stats,
            ));
            match job.file.source {
                SourceKind::Codex => {
                    codex_state.current_model = base_cache.codex_last_model.clone();
                    codex_state.previous_totals = base_cache.codex_last_totals;
                }
                SourceKind::Claude => {
                    claude_state =
                        ClaudeDedupeState::with_seed(base_cache.claude_recent_keys.clone());
                }
                SourceKind::Gemini | SourceKind::OpenCode | SourceKind::Grok => {}
            }
        } else {
            let _ = reader.seek(SeekFrom::Start(0));
        }
    }

    if job.file.source == SourceKind::Gemini {
        let ext = job
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext.eq_ignore_ascii_case("db") {
            return parse_antigravity_db_file(job, filter, timezone, pricing, stats);
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
            return parse_opencode_db_file(job, filter, timezone, pricing, stats);
        }
        return parse_opencode_message_file(job, filter, timezone, pricing, stats);
    }

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

        let parsed = match job.file.source {
            SourceKind::Claude => {
                parse_claude_usage_line(&line, pricing, &mut claude_state, global_claude_dedupe)
            }
            SourceKind::Codex => parse_codex_usage_line(&line, &mut codex_state, pricing),
            SourceKind::Gemini => parse_gemini_usage_line(&line, pricing),
            SourceKind::OpenCode => unreachable!("OpenCode is parsed as whole-file JSON"),
            SourceKind::Grok => parse_grok_usage_line(&line, pricing),
        };

        let mut parsed = match parsed {
            ParseLineResult::Parsed(parsed) => parsed,
            ParseLineResult::InvalidJson => {
                lines_invalid_json += 1;
                continue;
            }
            ParseLineResult::MissingUsage => {
                lines_missing_usage += 1;
                continue;
            }
        };

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

        cached_events.push(cached_usage_event(&parsed.event));

        let day = local_date(parsed.event.timestamp, timezone);
        if !filter.allows(day) {
            lines_filtered += 1;
            continue;
        }

        local_events.push(parsed.event);
        lines_parsed += 1;
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
        claude_recent_keys: claude_state.snapshot(),
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
    let limit = 2048.min(line.len());
    let mut boundary = limit;
    while boundary > 0 && !line.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let scan_window = &line[..boundary];
    !markers.iter().any(|marker| scan_window.contains(marker))
}

fn provider_fast_line_markers(source: SourceKind) -> &'static [&'static str] {
    match source {
        SourceKind::Codex => &[
            "\"type\":\"event_msg\"",
            "\"type\": \"event_msg\"",
            "\"type\":\"turn_context\"",
            "\"type\": \"turn_context\"",
        ],
        SourceKind::Claude => &["\"type\":\"assistant\"", "\"type\": \"assistant\""],
        SourceKind::Gemini => &[
            "\"type\":\"gemini\"",
            "\"type\": \"gemini\"",
            "\"tokens\":",
            "\"tokens\": ",
        ],
        SourceKind::OpenCode => &[],
        SourceKind::Grok => &[
            "\"sessionUpdate\":\"turn_completed\"",
            "\"sessionUpdate\": \"turn_completed\"",
            "\"msg\":\"shell.turn.inference_done\"",
            "\"msg\": \"shell.turn.inference_done\"",
        ],
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
        SourceKind::Grok => {
            // `<encoded_cwd>/<sessionID>/updates.jsonl` -> `<sessionID>`
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
        SourceKind::Claude => raw_project.map(|p| decode_claude_project_dir(&p)),
        SourceKind::Codex => extract_codex_project_from_file(&file.path).or(raw_project),
        SourceKind::Gemini => None,
        SourceKind::OpenCode => None,
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
            SourceKind::OpenCode => {}
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

pub(super) fn parse_claude_usage_line(
    line: &str,
    pricing: &PricingTable,
    dedupe_state: &mut ClaudeDedupeState,
    global_dedupe: &ClaudeGlobalDedupe,
) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    if get_value(&value, "type")
        .and_then(Value::as_str)
        .is_some_and(|entry_type| entry_type != "assistant")
    {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };
    let Some(model) = extract_model(&value, SourceKind::Claude) else {
        return ParseLineResult::MissingUsage;
    };

    let message_id = get_value(&value, "message.id")
        .and_then(Value::as_str)
        .or_else(|| get_value(&value, "messageId").and_then(Value::as_str));
    let request_id = get_value(&value, "requestId")
        .and_then(Value::as_str)
        .or_else(|| get_value(&value, "request_id").and_then(Value::as_str));
    if let Some(message_id) = message_id {
        let key = request_id
            .map(|r| format!("{message_id}:{r}"))
            .unwrap_or_else(|| message_id.to_string());
        if !dedupe_state.insert(key.clone()) {
            return ParseLineResult::MissingUsage;
        }
        if !global_dedupe.insert(&key) {
            return ParseLineResult::MissingUsage;
        }
    }

    let usage = UsageAccumulator {
        input_tokens: extract_u64(
            &value,
            &[
                "message.usage.input_tokens",
                "usage.input_tokens",
                "usage.inputTokens",
                "input_tokens",
                "inputTokens",
            ],
        )
        .unwrap_or(0),
        cache_creation_input_tokens: extract_u64(
            &value,
            &[
                "message.usage.cache_creation_input_tokens",
                "usage.cache_creation_input_tokens",
                "usage.cacheCreationInputTokens",
                "cache_creation_input_tokens",
                "cacheCreationInputTokens",
            ],
        )
        .unwrap_or(0),
        cache_read_input_tokens: extract_u64(
            &value,
            &[
                "message.usage.cache_read_input_tokens",
                "usage.cache_read_input_tokens",
                "usage.cached_input_tokens",
                "usage.cachedInputTokens",
                "cache_read_input_tokens",
                "cached_input_tokens",
                "cachedInputTokens",
            ],
        )
        .unwrap_or(0),
        output_tokens: extract_u64(
            &value,
            &[
                "message.usage.output_tokens",
                "usage.output_tokens",
                "usage.outputTokens",
                "output_tokens",
                "outputTokens",
            ],
        )
        .unwrap_or(0),
        reasoning_output_tokens: extract_u64(
            &value,
            &[
                "message.usage.reasoning_output_tokens",
                "usage.reasoning_output_tokens",
                "usage.reasoningOutputTokens",
                "usage.output_tokens_details.reasoning_tokens",
                "output_tokens_details.reasoning_tokens",
            ],
        )
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
            source: SourceKind::Claude,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
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

/// Parse one line of Grok session (`updates.jsonl`) or unified log (`unified.jsonl`).
///
/// Supports:
/// 1. `~/.grok/sessions/<encoded_cwd>/<session_id>/updates.jsonl` with `"sessionUpdate":"turn_completed"`
///    containing explicit per-model breakdowns, exact token metrics, and costUsdTicks.
/// 2. `~/.grok/logs/unified.jsonl` with `"msg":"shell.turn.inference_done"` for fallback.
pub(super) fn parse_grok_usage_line(line: &str, pricing: &PricingTable) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    // Format 1: updates.jsonl line with turn_completed
    let is_turn_completed = get_value(&value, "params.update.sessionUpdate")
        .or_else(|| get_value(&value, "update.sessionUpdate"))
        .and_then(Value::as_str)
        == Some("turn_completed");

    if is_turn_completed {
        let timestamp = extract_timestamp(&value)
            .or_else(|| get_value(&value, "_meta.agentTimestampMs").and_then(parse_timestamp_value))
            .or_else(|| get_value(&value, "timestamp").and_then(parse_timestamp_value));
        let Some(timestamp) = timestamp else {
            return ParseLineResult::MissingUsage;
        };

        let session =
            extract_string(&value, &["params.sessionId", "sessionId"]).unwrap_or_default();
        let usage_obj =
            get_value(&value, "params.update.usage").or_else(|| get_value(&value, "update.usage"));

        let Some(usage_val) = usage_obj else {
            return ParseLineResult::MissingUsage;
        };

        let input_tokens = extract_u64(usage_val, &["inputTokens"]).unwrap_or(0);
        let cached_read_tokens = extract_u64(usage_val, &["cachedReadTokens"]).unwrap_or(0);
        let output_tokens = extract_u64(usage_val, &["outputTokens"]).unwrap_or(0);
        let reasoning_tokens = extract_u64(usage_val, &["reasoningTokens"]).unwrap_or(0);
        let cost_usd_ticks = extract_u64(usage_val, &["costUsdTicks"]).unwrap_or(0);

        let usage = UsageAccumulator {
            input_tokens: input_tokens.saturating_sub(cached_read_tokens),
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: cached_read_tokens,
            output_tokens,
            reasoning_output_tokens: reasoning_tokens,
            cost_usd: 0.0,
        };

        if usage.total_tokens() == 0 {
            return ParseLineResult::MissingUsage;
        }

        // Determine model from modelUsage map or fallback
        let model = if let Some(model_usage) =
            get_value(usage_val, "modelUsage").and_then(Value::as_object)
        {
            model_usage
                .iter()
                .max_by_key(|(_, v)| extract_u64(v, &["totalTokens", "inputTokens"]).unwrap_or(0))
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| crate::types::GROK_DEFAULT_MODEL_FALLBACK.to_string())
        } else {
            extract_string(usage_val, &["model", "model_id"])
                .unwrap_or_else(|| crate::types::GROK_DEFAULT_MODEL_FALLBACK.to_string())
        };

        let (cost_usd, used_unknown_pricing) = if cost_usd_ticks > 0 {
            ((cost_usd_ticks as f64) / 1e9, false)
        } else {
            match pricing.estimate_cost(&model, usage) {
                Some(v) => (v, false),
                None => (0.0, true),
            }
        };

        return ParseLineResult::Parsed(ParsedLine {
            event: UsageEvent {
                timestamp,
                source: SourceKind::Grok,
                model,
                session,
                project: None,
                file_path: String::new(),
                usage: UsageAccumulator { cost_usd, ..usage },
            },
            used_unknown_pricing,
        });
    }

    // Format 2: unified.jsonl line with shell.turn.inference_done
    if get_value(&value, "msg").and_then(Value::as_str) != Some("shell.turn.inference_done") {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = get_value(&value, "ts").and_then(parse_timestamp_value) else {
        return ParseLineResult::MissingUsage;
    };

    let Some(session) = extract_string(&value, &["sid"]) else {
        return ParseLineResult::MissingUsage;
    };

    let prompt_tokens = extract_u64(&value, &["ctx.prompt_tokens"]).unwrap_or(0);
    let cached_prompt_tokens = extract_u64(&value, &["ctx.cached_prompt_tokens"]).unwrap_or(0);
    let completion_tokens = extract_u64(&value, &["ctx.completion_tokens"]).unwrap_or(0);
    let reasoning_tokens = extract_u64(&value, &["ctx.reasoning_tokens"]).unwrap_or(0);

    let usage = UsageAccumulator {
        input_tokens: prompt_tokens.saturating_sub(cached_prompt_tokens),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached_prompt_tokens,
        output_tokens: completion_tokens,
        reasoning_output_tokens: reasoning_tokens,
        cost_usd: 0.0,
    };

    let total_tokens = usage.total_tokens();
    if total_tokens == 0 {
        return ParseLineResult::MissingUsage;
    }

    let model = crate::types::GROK_DEFAULT_MODEL_FALLBACK.to_string();
    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };

    ParseLineResult::Parsed(ParsedLine {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Grok,
            model,
            session,
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
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

    let day = local_date(timestamp, timezone);
    if !filter.allows(day) {
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

    let event = UsageEvent {
        timestamp,
        source: SourceKind::OpenCode,
        model,
        session,
        project,
        file_path: job.file.path.display().to_string(),
        usage: UsageAccumulator { cost_usd, ..usage },
    };

    Some(ParsedFileOutput {
        events: vec![event.clone()],
        cache_entry: CachedFileEntry {
            fingerprint: job.fingerprint,
            stats: CachedFileStats {
                lines_total: 1,
                lines_invalid_json: 0,
                lines_missing_usage: 0,
                lines_unknown_pricing: if used_unknown_pricing { 1 } else { 0 },
            },
            events: vec![cached_usage_event(&event)],
            parsed_offset: job.fingerprint.size,
            codex_last_model: None,
            codex_last_totals: None,
            claude_recent_keys: vec![],
        },
    })
}

fn filter_bounds_utc_millis(filter: DateFilter, tz: &TimeZoneMode) -> Option<(i64, i64)> {
    let since = filter.since?;
    let until = filter.until?;

    let start_local = since.and_hms_opt(0, 0, 0)?;
    let end_local = until
        .checked_add_days(chrono::Days::new(1))?
        .and_hms_opt(0, 0, 0)?;

    let start_utc = match tz {
        TimeZoneMode::Utc => DateTime::<Utc>::from_naive_utc_and_offset(start_local, Utc),
        TimeZoneMode::Local => Local
            .from_local_datetime(&start_local)
            .earliest()?
            .with_timezone(&Utc),
        TimeZoneMode::Named(tz) => tz
            .from_local_datetime(&start_local)
            .earliest()?
            .with_timezone(&Utc),
    };
    let end_utc = match tz {
        TimeZoneMode::Utc => DateTime::<Utc>::from_naive_utc_and_offset(end_local, Utc),
        TimeZoneMode::Local => Local
            .from_local_datetime(&end_local)
            .earliest()?
            .with_timezone(&Utc),
        TimeZoneMode::Named(tz) => tz
            .from_local_datetime(&end_local)
            .earliest()?
            .with_timezone(&Utc),
    };

    Some((start_utc.timestamp_millis(), end_utc.timestamp_millis()))
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

    let mut stmt = conn.prepare("SELECT data FROM gen_metadata").ok()?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)).ok()?;

    let mut events = Vec::new();
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

        if gen_event.timestamp_secs <= 0 {
            lines_missing_usage += 1;
            continue;
        }

        let timestamp = if gen_event.timestamp_secs > 10_000_000_000 {
            DateTime::from_timestamp_millis(gen_event.timestamp_secs)
        } else {
            DateTime::from_timestamp(gen_event.timestamp_secs, 0)
        };
        let Some(timestamp) = timestamp else {
            lines_missing_usage += 1;
            continue;
        };

        if !filter.allows(local_date(timestamp, timezone)) {
            lines_filtered += 1;
            continue;
        }

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
            file_path: path.to_string_lossy().to_string(),
            usage: UsageAccumulator { cost_usd, ..usage },
        };
        events.push(event);

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

    let cached_events = events.iter().map(cached_usage_event).collect();

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
            codex_last_model: None,
            codex_last_totals: None,
            claude_recent_keys: vec![],
        },
    })
}

fn parse_opencode_db_file(
    job: FileParseJob,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
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

    let bounds = filter_bounds_utc_millis(filter, timezone);
    let (sql, params): (&str, Vec<i64>) = if let Some((start_ms, end_ms)) = bounds {
        (
            "SELECT m.id, m.session_id, s.id, m.time_created, m.data, s.directory, p.worktree \
             FROM message m \
             LEFT JOIN session s ON m.session_id = s.id \
             LEFT JOIN project p ON s.project_id = p.id \
             WHERE m.time_created >= ?1 AND m.time_created < ?2 \
             ORDER BY m.time_created ASC",
            vec![start_ms, end_ms],
        )
    } else {
        (
            "SELECT m.id, m.session_id, s.id, m.time_created, m.data, s.directory, p.worktree \
             FROM message m \
             LEFT JOIN session s ON m.session_id = s.id \
             LEFT JOIN project p ON s.project_id = p.id \
             ORDER BY m.time_created ASC",
            vec![],
        )
    };

    let mut lines_total = 0usize;
    let mut lines_parsed = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;

    let mut stmt = conn.prepare(sql).ok()?;
    let mut rows = if params.len() == 2 {
        stmt.query([params[0], params[1]]).ok()?
    } else {
        stmt.query([]).ok()?
    };

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

        let day = local_date(timestamp, timezone);
        if !filter.allows(day) {
            lines_filtered += 1;
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

        let event = UsageEvent {
            timestamp,
            source: SourceKind::OpenCode,
            model: model.clone(),
            session,
            project,
            file_path: format!("{}#{}", path.display(), message_id),
            usage: UsageAccumulator { cost_usd, ..usage },
        };
        cached_events.push(cached_usage_event(&event));
        events.push(event);
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
            codex_last_model: None,
            codex_last_totals: None,
            claude_recent_keys: vec![],
        },
    })
}

pub(super) fn parse_codex_usage_line(
    line: &str,
    state: &mut CodexParseState,
    pricing: &PricingTable,
) -> ParseLineResult {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseLineResult::InvalidJson,
    };

    let Some(entry_type) = get_value(&value, "type").and_then(|v| v.as_str()) else {
        return ParseLineResult::MissingUsage;
    };

    if entry_type == "turn_context" {
        if let Some(payload) = get_value(&value, "payload")
            && let Some(model) = extract_codex_model(payload)
        {
            state.current_model = Some(model);
            state.current_model_is_fallback = false;
        }
        return ParseLineResult::MissingUsage;
    }

    if entry_type != "event_msg" {
        return ParseLineResult::MissingUsage;
    }

    let Some(payload_type) = get_value(&value, "payload.type").and_then(|v| v.as_str()) else {
        return ParseLineResult::MissingUsage;
    };
    if payload_type != "token_count" {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };

    let info = get_value(&value, "payload.info");
    let last_usage = info.and_then(|v| get_value(v, "last_token_usage"));
    let total_usage = info.and_then(|v| get_value(v, "total_token_usage"));

    let parsed_last = last_usage.and_then(parse_codex_raw_usage);
    let parsed_total = total_usage.and_then(parse_codex_raw_usage);

    let raw_delta = if let Some(last) = parsed_last {
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
        return ParseLineResult::MissingUsage;
    };
    if raw_delta.is_zero() {
        return ParseLineResult::MissingUsage;
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

    let usage = codex_delta_to_usage(raw_delta);

    let (cost_usd, used_unknown_pricing) = match pricing.estimate_cost(&model, usage) {
        Some(v) => (v, false),
        None => (0.0, true),
    };

    ParseLineResult::Parsed(ParsedLine {
        event: UsageEvent {
            timestamp,
            source: SourceKind::Codex,
            model,
            session: String::new(),
            project: None,
            file_path: String::new(),
            usage: UsageAccumulator { cost_usd, ..usage },
        },
        used_unknown_pricing,
    })
}

pub(super) fn extract_model(value: &Value, source: SourceKind) -> Option<String> {
    extract_string(value, provider_model_paths(source))
}

fn provider_model_paths(source: SourceKind) -> &'static [&'static str] {
    match source {
        SourceKind::Claude => &[
            "message.model",
            "usage.model",
            "model",
            "payload.model",
            "message.metadata.model",
        ],
        SourceKind::Codex => &[
            "payload.info.model",
            "payload.info.current_model",
            "payload.model",
            "model",
        ],
        SourceKind::Gemini => &["model"],
        SourceKind::OpenCode => &["model.modelID", "modelID", "model"],
        // Grok's unified.jsonl never records a per-turn model name; the
        // fallback in `parse_grok_usage_line` is used unconditionally instead.
        SourceKind::Grok => &["params.update.usage.model", "update.usage.model", "model"],
    }
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

    if let Some(openrouter_exact) = load_openrouter_pricing_with_cache(!offline).await {
        table.merge_exact(openrouter_exact);
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
                save_openrouter_pricing_cache(path, now, &exact);
            }
            Some(exact)
        }
        Err(_) => cached.map(|cache| cache.exact),
    }
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
    // Use the same opener as the real cache (WAL + busy_timeout); a bare
    // Connection::open can miss rows still living in the -wal file.
    let entries = if exists {
        open_cache_db(&path)
            .ok()
            .and_then(|conn| {
                conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
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

fn open_cache_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // WAL keeps concurrent readers (e.g. statusline alongside an interactive run)
    // from blocking; NORMAL sync is durable enough for a rebuildable cache.
    let _ = conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA mmap_size=268435456;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-64000;",
    );
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS files (cache_key TEXT PRIMARY KEY, entry TEXT NOT NULL);",
    )?;
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

/// One-time import of the legacy JSON cache into an empty SQLite DB. The JSON
/// file is removed afterwards (whether imported or stale) so it's never retried.
fn migrate_legacy_json_if_present(conn: &Connection, pricing_key: &str) {
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap_or(0);
    if existing > 0 {
        return;
    }
    let Some(json_path) = legacy_json_cache_path() else {
        return;
    };
    let _ = import_legacy_json(conn, pricing_key, &json_path);
    // Remove the legacy file whether imported or stale, so it's never retried.
    let _ = std::fs::remove_file(&json_path);
}

/// Import a legacy JSON cache file into the (empty) SQLite `files` table when its
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
        let mut stmt =
            tx.prepare("INSERT OR REPLACE INTO files (cache_key, entry) VALUES (?1, ?2)")?;
        for (key, entry) in &store.files {
            if let Ok(json) = serde_json::to_string(entry) {
                stmt.execute(params![key, json])?;
            }
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
        // Stale or first-time cache: clear and start fresh (DB now empty).
        conn.execute("DELETE FROM files", [])?;
        cache_meta_set(&conn, "version", &INCREMENTAL_CACHE_VERSION.to_string())?;
        cache_meta_set(&conn, "pricing_key", pricing_key)?;
        return Ok(IncrementalCacheStore::new(pricing_key.to_string()));
    }

    let mut files = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT cache_key, entry FROM files")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let entry_bytes = row.get_ref(1)?.as_bytes()?;
            if let Ok(entry) = serde_json::from_slice::<CachedFileEntry>(entry_bytes) {
                files.insert(key, entry);
            }
        }
    }

    Ok(IncrementalCacheStore {
        version: INCREMENTAL_CACHE_VERSION,
        pricing_key: pricing_key.to_string(),
        files,
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
        tx.execute("DELETE FROM files", [])?;
        let mut stmt =
            tx.prepare("INSERT OR REPLACE INTO files (cache_key, entry) VALUES (?1, ?2)")?;
        for (key, entry) in &store.files {
            if let Ok(json) = serde_json::to_string(entry) {
                stmt.execute(params![key, json])?;
            }
        }
    } else {
        {
            let mut up =
                tx.prepare("INSERT OR REPLACE INTO files (cache_key, entry) VALUES (?1, ?2)")?;
            for key in &store.changed {
                if let Some(entry) = store.files.get(key)
                    && let Ok(json) = serde_json::to_string(entry)
                {
                    up.execute(params![key, json])?;
                }
            }
        }
        {
            let mut del = tx.prepare("DELETE FROM files WHERE cache_key = ?1")?;
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

pub(super) fn hydrate_cached_events(
    file: &DiscoveredFile,
    cached: &CachedFileEntry,
    filter: DateFilter,
    timezone: &TimeZoneMode,
    pricing: &PricingTable,
    stats: &ParseStatsAtomic,
) -> Vec<UsageEvent> {
    stats
        .lines_total
        .fetch_add(cached.stats.lines_total, Ordering::Relaxed);
    stats
        .lines_invalid_json
        .fetch_add(cached.stats.lines_invalid_json, Ordering::Relaxed);
    stats
        .lines_missing_usage
        .fetch_add(cached.stats.lines_missing_usage, Ordering::Relaxed);
    // NB: lines_unknown_pricing is NOT taken from the cached stats — it depends
    // on the CURRENT pricing table, so we recompute it live below (pre-filter,
    // matching the parser, which counts unknown pricing before the date filter).

    let (session, project) = derive_session_meta(file);
    let file_path = file.path.display().to_string();
    let mut filtered = 0usize;
    let mut parsed = 0usize;
    let mut unknown_pricing = 0usize;
    let mut events = Vec::with_capacity(cached.events.len());

    for cached_event in &cached.events {
        // Re-price against the CURRENT table, mirroring the parser EXACTLY so a
        // cached event never drifts from a freshly-parsed one: the parser uses
        // `estimate_cost(...).map(..).unwrap_or((0.0, unknown))`, i.e. an unknown
        // model is $0 (not the stale cached cost). Count unknowns pre-filter.
        let mut usage = cached_event.usage;
        match pricing.estimate_cost(&cached_event.model, usage) {
            Some(cost) => usage.cost_usd = cost,
            None => {
                usage.cost_usd = 0.0;
                unknown_pricing += 1;
            }
        }

        let day = local_date(cached_event.timestamp, timezone);
        if !filter.allows(day) {
            filtered += 1;
            continue;
        }
        parsed += 1;
        events.push(UsageEvent {
            timestamp: cached_event.timestamp,
            source: file.source,
            model: cached_event.model.clone(),
            session: cached_event
                .session
                .clone()
                .unwrap_or_else(|| session.clone()),
            project: cached_event.project.clone().or_else(|| project.clone()),
            file_path: cached_event
                .file_path
                .clone()
                .unwrap_or_else(|| file_path.clone()),
            usage,
        });
    }

    stats.lines_filtered.fetch_add(filtered, Ordering::Relaxed);
    stats.lines_parsed.fetch_add(parsed, Ordering::Relaxed);
    stats
        .lines_unknown_pricing
        .fetch_add(unknown_pricing, Ordering::Relaxed);

    events
}

fn cached_usage_event(event: &UsageEvent) -> CachedUsageEvent {
    CachedUsageEvent {
        timestamp: event.timestamp,
        model: event.model.clone(),
        usage: event.usage,
        session: Some(event.session.clone()),
        project: event.project.clone(),
        file_path: Some(event.file_path.clone()),
    }
}

/// Cache invalidation key — deliberately **independent of pricing**.
///
/// Cached events store token counts and are re-priced at hydration
/// (see [`hydrate_cached_events`]), so a pricing refresh no longer wipes the
/// cache. Previously this embedded the entire pricing table (800+ models), so
/// every 6h OpenRouter refresh — or any online/offline flip — changed the key
/// and forced a full reparse, making the cache nearly useless. Bump the marker
/// only when the parse/cache-entry *semantics* change. `v3` = re-price-on-hydrate.
pub(super) fn pricing_cache_key(_pricing: &PricingTable) -> String {
    "estimate-v3-reprice".to_string()
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
            stats: CachedFileStats::default(),
            events: Vec::new(),
            parsed_offset: size,
            codex_last_model: None,
            codex_last_totals: None,
            claude_recent_keys: Vec::new(),
        }
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
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
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

        let result = parse_grok_usage_line(sample, &pricing);
        let ParseLineResult::Parsed(parsed) = result else {
            panic!("Expected parsed result");
        };

        assert_eq!(parsed.event.source, SourceKind::Grok);
        assert_eq!(parsed.event.model, "grok-4.6-build");
        assert_eq!(parsed.event.session, "019fa4ff-8769-76c1-b662-ab0d9141d183");
        assert_eq!(parsed.event.usage.input_tokens, 659466 - 608640);
        assert_eq!(parsed.event.usage.cache_read_input_tokens, 608640);
        assert_eq!(parsed.event.usage.output_tokens, 11143);
        assert_eq!(parsed.event.usage.reasoning_output_tokens, 6744);
        assert!((parsed.event.usage.cost_usd - 1.9484).abs() < 1e-4);
    }

    #[test]
    fn parse_grok_unified_log_turn_inference_done() {
        let pricing = PricingTable::default();
        let sample = r#"{"ts":"2026-08-25T14:12:31.661Z","src":"shell","pid":49284,"ver":"1.0.5","lvl":"info","sid":"01a03943-f501-70f3-a587-28e468a24112","msg":"shell.turn.inference_done","ctx":{"prompt_tokens":1000,"cached_prompt_tokens":800,"completion_tokens":200,"reasoning_tokens":50}}"#;

        let result = parse_grok_usage_line(sample, &pricing);
        let ParseLineResult::Parsed(parsed) = result else {
            panic!("Expected parsed result");
        };

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
}

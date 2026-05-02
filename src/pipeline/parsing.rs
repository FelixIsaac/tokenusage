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
use rusqlite::{Connection, OpenFlags};
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
    let mut seen_cache_keys = HashSet::with_capacity(files.len());
    let mut parse_jobs = Vec::new();
    let mut events = Vec::new();

    for file in files {
        let key = cache_file_key(&file.path);
        seen_cache_keys.insert(key.clone());

        let Some(fingerprint) = read_file_fingerprint(&file.path) else {
            parse_jobs.push(FileParseJob {
                file: file.clone(),
                cache_key: key,
                fingerprint: FileFingerprint {
                    size: 0,
                    modified_unix_secs: 0,
                    modified_unix_nanos: 0,
                },
                strategy: ParseStrategy::Full,
            });
            continue;
        };

        if cache_enabled && let Some(cached) = cache_store.files.get(&key) {
            if cached.fingerprint == fingerprint {
                events.extend(hydrate_cached_events(
                    file, cached, filter, timezone, &stats,
                ));
                continue;
            }
            if can_incremental_parse(cached, fingerprint) {
                parse_jobs.push(FileParseJob {
                    file: file.clone(),
                    cache_key: key,
                    fingerprint,
                    strategy: ParseStrategy::Incremental {
                        base_cache: cached.clone(),
                    },
                });
                continue;
            }
        }

        parse_jobs.push(FileParseJob {
            file: file.clone(),
            cache_key: key,
            fingerprint,
            strategy: ParseStrategy::Full,
        });
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
            cache_store.files.insert(key, entry);
            cache_dirty = true;
        }

        let before = cache_store.files.len();
        cache_store
            .files
            .retain(|path, _| seen_cache_keys.contains(path));
        if cache_store.files.len() != before {
            cache_dirty = true;
        }
    }

    if sort_events {
        events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
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
            save_incremental_cache(path, &self.cache_store);
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
        save_incremental_cache(path, &cache_store);
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

fn provider_registry() -> [ProviderSpec; 4] {
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
            accepted_exts: &["jsonl"],
            enabled: |common| !common.no_gemini,
            roots: gemini_source_roots,
        },
        ProviderSpec {
            kind: SourceKind::OpenCode,
            accepted_exts: &["json", "db"],
            enabled: |common| !common.no_opencode,
            roots: opencode_source_roots,
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
        return vec![home.join(".gemini").join("tmp")];
    }
    common
        .gemini_data_dir
        .iter()
        .map(|p| expand_user_path(p))
        .collect()
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

        out.push(DiscoveredFile {
            source: kind,
            root: root.to_path_buf(),
            path: normalized_discovered_path(path),
        });
    }

    out
}

pub(super) fn dedupe_opencode_events(events: &mut Vec<UsageEvent>) {
    let mut seen = HashSet::new();
    events.retain(|event| {
        if event.source != SourceKind::OpenCode {
            return true;
        }
        if let Some((_, suffix)) = event.file_path.rsplit_once('#')
            && suffix.starts_with("msg_")
        {
            return seen.insert(suffix.to_string());
        }

        if let Some(file_name) = Path::new(&event.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            && let Some(stem) = file_name.strip_suffix(".json")
            && stem.starts_with("msg_")
        {
            return seen.insert(stem.to_string());
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
        seen.insert(fallback)
    });
}

fn provider_accepts_extension(kind: SourceKind, ext: &str) -> bool {
    let meta = provider_registry()
        .into_iter()
        .find(|spec| spec.kind == kind)
        .expect("provider metadata must exist");
    meta.accepted_exts
        .iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
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
        let mut seen = self.seen_keys.lock().expect("claude dedupe mutex poisoned");
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

    let mut reader = BufReader::new(input);
    let mut codex_state = CodexParseState::default();
    let mut claude_state = ClaudeDedupeState::default();
    let mut local_events = Vec::new();
    let mut cached_events = Vec::new();
    let mut line = String::new();
    let mut lines_total = 0usize;
    let mut lines_invalid_json = 0usize;
    let mut lines_missing_usage = 0usize;
    let mut lines_unknown_pricing = 0usize;
    let mut lines_filtered = 0usize;
    let mut lines_parsed = 0usize;
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
                &job.file,
                &base_cache,
                filter,
                timezone,
                stats,
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
                SourceKind::Gemini | SourceKind::OpenCode => {}
            }
        } else {
            let _ = reader.seek(SeekFrom::Start(0));
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

        cached_events.push(CachedUsageEvent {
            timestamp: parsed.event.timestamp,
            model: parsed.event.model.clone(),
            usage: parsed.event.usage,
        });

        let day = local_date(parsed.event.timestamp, timezone);
        if !filter.allows(day) {
            lines_filtered += 1;
            continue;
        }

        parsed.event.session = session.clone();
        parsed.event.project = project.clone();
        parsed.event.file_path = file_path.clone();

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
    !markers.is_empty() && !markers.iter().any(|marker| line.contains(marker))
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
    if let (Some(message_id), Some(request_id)) = (message_id, request_id) {
        let key = format!("{message_id}:{request_id}");
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

    // Gemini CLI chat logs contain many record types; we only care about
    // assistant completions with token accounting.
    if get_value(&value, "type")
        .and_then(Value::as_str)
        .is_some_and(|entry_type| entry_type != "gemini")
    {
        return ParseLineResult::MissingUsage;
    }

    let Some(timestamp) = extract_timestamp(&value) else {
        return ParseLineResult::MissingUsage;
    };

    let Some(model) = extract_string(&value, &["model"]) else {
        return ParseLineResult::MissingUsage;
    };

    let usage = UsageAccumulator {
        input_tokens: extract_u64(&value, &["tokens.input"]).unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: extract_u64(&value, &["tokens.cached"]).unwrap_or(0),
        output_tokens: extract_u64(&value, &["tokens.output"]).unwrap_or(0),
        reasoning_output_tokens: extract_u64(&value, &["tokens.thoughts"]).unwrap_or(0),
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

    let session = extract_string(&value, &["sessionID"]).unwrap_or_default();
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
            events: vec![CachedUsageEvent {
                timestamp: event.timestamp,
                model: event.model.clone(),
                usage: event.usage,
            }],
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
    .ok()?;

    let mut events = Vec::new();
    let mut cached_events = Vec::new();

    let bounds = filter_bounds_utc_millis(filter, timezone);
    let (sql, params): (&str, Vec<i64>) = if let Some((start_ms, end_ms)) = bounds {
        (
            "SELECT m.id, m.session_id, m.time_created, m.data, s.directory, p.worktree \
             FROM message m \
             JOIN session s ON m.session_id = s.id \
             JOIN project p ON s.project_id = p.id \
             WHERE m.time_created >= ?1 AND m.time_created < ?2 \
             ORDER BY m.time_created ASC",
            vec![start_ms, end_ms],
        )
    } else {
        (
            "SELECT m.id, m.session_id, m.time_created, m.data, s.directory, p.worktree \
             FROM message m \
             JOIN session s ON m.session_id = s.id \
             JOIN project p ON s.project_id = p.id \
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
        let session_id: String = row.get(1).ok()?;
        let time_created: i64 = row.get(2).ok()?;
        let data: String = row.get(3).ok()?;
        if data.len() > MAX_JSON_LINE_BYTES {
            lines_invalid_json += 1;
            continue;
        }
        let session_dir: String = row.get(4).ok()?;
        let project_worktree: String = row.get(5).ok()?;

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

        let project = Path::new(&session_dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .or_else(|| {
                Path::new(&project_worktree)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            });

        let event = UsageEvent {
            timestamp,
            source: SourceKind::OpenCode,
            model: model.clone(),
            session: session_id,
            project,
            file_path: format!("{}#{}", path.display(), message_id),
            usage: UsageAccumulator { cost_usd, ..usage },
        };
        cached_events.push(CachedUsageEvent {
            timestamp: event.timestamp,
            model: event.model.clone(),
            usage: event.usage,
        });
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
    Some(base.join("tokenusage").join("parse-cache-v2.json"))
}

pub(super) fn load_incremental_cache(path: &Path, pricing_key: &str) -> IncrementalCacheStore {
    let body = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return IncrementalCacheStore::new(pricing_key.to_string()),
    };

    let store: IncrementalCacheStore = match serde_json::from_slice(&body) {
        Ok(store) => store,
        Err(_) => return IncrementalCacheStore::new(pricing_key.to_string()),
    };

    if store.version != INCREMENTAL_CACHE_VERSION || store.pricing_key != pricing_key {
        return IncrementalCacheStore::new(pricing_key.to_string());
    }

    store
}

pub(super) fn save_incremental_cache(path: &Path, store: &IncrementalCacheStore) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(store) {
        let _ = std::fs::write(path, bytes);
    }
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
    stats
        .lines_unknown_pricing
        .fetch_add(cached.stats.lines_unknown_pricing, Ordering::Relaxed);

    let (session, project) = derive_session_meta(file);
    let file_path = file.path.display().to_string();
    let mut filtered = 0usize;
    let mut parsed = 0usize;
    let mut events = Vec::with_capacity(cached.events.len());

    for cached_event in &cached.events {
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
            session: session.clone(),
            project: project.clone(),
            file_path: file_path.clone(),
            usage: cached_event.usage,
        });
    }

    stats.lines_filtered.fetch_add(filtered, Ordering::Relaxed);
    stats.lines_parsed.fetch_add(parsed, Ordering::Relaxed);

    events
}

pub(super) fn pricing_cache_key(pricing: &PricingTable) -> String {
    let mut out = String::new();
    out.push_str("estimate-v2");
    out.push('|');

    let mut exact = pricing.exact.iter().collect::<Vec<_>>();
    exact.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (model, rate) in exact {
        out.push_str(model);
        out.push(':');
        out.push_str(&pricing_rate_key(rate));
        out.push('|');
    }

    out.push('#');
    for (prefix, rate) in &pricing.prefixes {
        out.push_str(prefix);
        out.push(':');
        out.push_str(&pricing_rate_key(rate));
        out.push('|');
    }

    out
}

pub(super) fn pricing_rate_key(rate: &PricingRate) -> String {
    format!(
        "{:.8},{:.8},{:.8},{:.8},{:.8},{},{},{},{},{},{}",
        rate.input_per_million,
        rate.output_per_million,
        rate.cache_creation_per_million,
        rate.cache_read_per_million,
        rate.reasoning_output_per_million,
        rate.tier_threshold_tokens
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        rate.input_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.output_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.cache_creation_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.cache_read_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
        rate.reasoning_output_above_per_million
            .map(|v| format!("{v:.8}"))
            .unwrap_or_else(|| "-".to_string()),
    )
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

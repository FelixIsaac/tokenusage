# AI Coding Provider Session Schemas & Pricing Rules

Reference documentation of local session-log formats, token-accounting semantics, deduplication rules, service tiers, and cost-estimation rules across the 8 major AI coding agents/providers studied alongside [`tokenusage`](../../README.md) (`tu`) and [T3 Code](https://github.com/pingdotgg/t3code).

All file paths, session identifiers, model examples, and token counts in these documents are **synthetic**.

## Provider Matrix

| Provider | Storage Format | Default Discovery Root(s) | Input Counter Semantics | Reasoning Tokens | Dedupe Strategy | Provider-Reported Cost | `tu` Support |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Claude Code** | JSONL | `~/.claude/projects/**/*.jsonl` | Disjoint (`input_tokens` excludes cache read/write) | Folded inside `output_tokens` | Max-merge per token channel across `message.id:requestId` | Optional `costUSD` | ✅ (`claude`) |
| **OpenAI Codex** | JSONL | `$CODEX_HOME/sessions/**/*.jsonl`, `$CODEX_HOME/archived_sessions/**/*.jsonl` | **Inclusive** (`input_tokens` includes `cached_input_tokens` + `cache_write_input_tokens`) | Included inside `output_tokens` | Delta over `total_token_usage`, unchanged-total guard, fork anchor suppression, `token_usage_record` ledger | Estimated | ✅ (`codex`) |
| **Grok Build (xAI)** | JSONL | `~/.grok/sessions/**/updates.jsonl` (legacy: `~/.grok/logs/unified.jsonl`) | **Inclusive** (`inputTokens` includes `cachedReadTokens` + `cacheCreationTokens`) | Included inside `outputTokens` (clamped) | First-seen per `${sessionId}:${prompt_id}:${timestampMs}:${model}` | `costUsdTicks / 1e10` (split across `modelUsage`) | ✅ (`grok`) |
| **Antigravity & Gemini CLI** | Protobuf + SQLite (`state.vscdb`) / JSON | `~/.gemini/antigravity*/conversations/*.pb`, `~/.gemini/tmp/**/chats/*.jsonl` | Disjoint (`tokens.cached` separate from `tokens.input`) | Tracked separately (additive in Gemini CLI legacy JSON; Antigravity uses proto input/cached/output) | Whole-file / row replacement per `.pb` or session | Estimated (`rateModel` strips variant suffixes) | ✅ (`gemini` / `agy`) |
| **OpenCode** | SQLite (`opencode.db`) + JSON | `$OPENCODE_DATA_DIR`, `$XDG_DATA_HOME/opencode`, `~/.local/share/opencode` | Disjoint (`tokens.input` excludes `cache.read` + `cache.write`) | Additive to raw `tokens.output` (`outputTokens = output + reasoning`) | `opencode:${messageId}` (DB wins over legacy JSON files) | `cost` (except non-free `$0` placeholders) | ✅ (`opencode`) |
| **Pi** | JSONL | `$PI_CODING_AGENT_SESSION_DIR`, `$PI_CODING_AGENT_DIR/sessions`, `~/.pi/agent/sessions/**/*.jsonl` | Disjoint (`input`, `cacheRead`, `cacheWrite`) | Included inside `output` (clamped) | First-seen per `["pi", id, timestamp, model]` | `usage.cost.total` when finite `> 0` | ✅ (`pi`) |
| **Muse Code** | JSONL | `$MUSE_HOME/sessions/**/session.jsonl`, `$XDG_DATA_HOME/muse/sessions/**/session.jsonl` | **Inclusive** (`input_tokens` includes `cache_read_tokens` + `cache_write_tokens`) | Included inside `output_tokens` (clamped) | First-seen per `source_run_record_id ?? entry.id` | Estimated | ✅ (`muse`) |
| **Cursor** | Remote Account Usage API (`cursor.com`) + local key/cache | Auth token via CLIProxy/keychain/credentials DB (no local token ledger in `~/.cursor`) | Disjoint (`inputTokens`, `cacheReadTokens`, `cacheWriteTokens`) | `0` (not broken out) | Boundary overlap reconciliation + occurrence index | `totalCents / 100` | Documented only (offline-first policy) |

## Cross-Cutting Pricing Semantics

1. **Disjoint vs. Inclusive Input Counters**:
   - **Claude**, **OpenCode**, **Gemini**, **Pi**, and **Cursor** report uncached input separately from cache reads and cache writes.
   - **Codex**, **Grok Build**, and **Muse Code** report `input` *inclusive* of cached read (and write) tokens: `uncached_input = max(0, input - cache_read - cache_write)`.
2. **1-Hour Prompt Cache Writes (`ephemeral_1h_input_tokens`)**:
   - Standard 5-minute Anthropic cache writes bill at `1.25x` base input (`cache_creation_input_token_cost`), while 1-hour TTL writes bill at `2.0x` base input (`cache_creation_input_token_cost_above_1hr`).
3. **Speed / Priority Tiers**:
   - Anthropic Fast Mode (`usage.speed == "fast"`) scales rates by `provider_specific_entry.fast` (e.g. `2.0x` or `6.0x` depending on model/generation).
   - OpenAI Priority Processing (`service_tier == "fast"` or `"priority"`) uses `_priority` rates (~`2.0x`), while `"flex"` / batch rates are separate (`_flex` / `_batches`).
   - Ultrafast (`_ultrafast`) maps to `ServiceSpeed::Ultrafast`.
4. **Long-Context Cliffs (`>200K` / `>272K` Prompt Tokens)**:
   - Models such as Gemini Pro / Grok / GPT-5.4 double (or step up) rates for the *entire* request when per-request `prompt_tokens = uncached_input + cache_read + cache_write` exceeds `tier.threshold_tokens`.
   - **Multi-call turn guard**: When an aggregated turn record (such as Grok Build's `turn_completed`) sums `modelCalls > 1` requests into a single entry without `costUsdTicks`, context-tier selection must evaluate `(prompt_tokens / model_calls) > threshold`, not the summed turn total.
5. **Pricing Sources**:
   - Primary table: [LiteLLM](https://github.com/BerriAI/litellm) (`model_prices_and_context_window.json`) for 1h cache writes, context tiers, and `_priority`/`_ultrafast`/`fast` multipliers (avoiding regional Bedrock/Vertex uplifts for bare names).
   - Secondary / fallback table: [OpenRouter](https://openrouter.ai) (`/api/v1/models`) + offline curated defaults.

## Credits

Session format documentation and edge-case validation were cross-referenced against [T3 Code (`pingdotgg/t3code`)](https://github.com/pingdotgg/t3code) (MIT License) alongside direct analysis of synthetic & anonymized schema shapes.

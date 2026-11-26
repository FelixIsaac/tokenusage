# OpenAI Codex (`codex`)

- **Default Paths**:
  - `$CODEX_HOME/sessions/**/*.jsonl` (defaults to `~/.codex/sessions/**/*.jsonl`)
  - `$CODEX_HOME/archived_sessions/**/*.jsonl`
  - `~/.config/codex/sessions/**/*.jsonl` and `~/.config/codex/archived_sessions/**/*.jsonl`
- **Format**: Event-stream JSONL per session (plus SQLite/thread metadata headers).

## Relevant Line Types

1. **`session_meta`** — session metadata and fork ancestry:
   ```json
   {
     "timestamp": "2026-10-01T12:00:00Z",
     "type": "session_meta",
     "payload": {
       "id": "00000000-0000-4000-8000-000000000001",
       "cwd": "/Users/dev/projects/example-app",
       "forked_from_id": "00000000-0000-4000-8000-000000000000"
     }
   }
   ```
2. **`turn_context`** — active model and working directory for subsequent events:
   ```json
   {
     "timestamp": "2026-10-01T12:00:01Z",
     "type": "turn_context",
     "payload": {
       "model": "gpt-5.4",
       "cwd": "/Users/dev/projects/example-app"
     }
   }
   ```
3. **`event_msg` (`thread_settings_applied`)** — updates active service tier (`"fast"` / `"priority"` -> Fast pricing, `"flex"`, or `"standard"`):
   ```json
   {
     "timestamp": "2026-10-01T12:00:02Z",
     "type": "event_msg",
     "payload": {
       "type": "thread_settings_applied",
       "thread_settings": {
         "service_tier": "fast"
       }
     }
   }
   ```
4. **`event_msg` (`token_count`)** — carries both per-request (`last_token_usage`) and cumulative session totals (`total_token_usage`):
   ```json
   {
     "timestamp": "2026-10-01T12:00:10Z",
     "type": "event_msg",
     "payload": {
       "type": "token_count",
       "info": {
         "last_token_usage": {
           "input_tokens": 12000,
           "cached_input_tokens": 10000,
           "output_tokens": 400,
           "reasoning_output_tokens": 150,
           "total_tokens": 12400
         },
         "total_token_usage": {
           "input_tokens": 12000,
           "cached_input_tokens": 10000,
           "output_tokens": 400,
           "reasoning_output_tokens": 150,
           "total_tokens": 12400
         }
       }
     }
   }
   ```
5. **`token_usage_record`** — explicit per-request usage record (newer Codex builds).

## Parsing Rules & Gotchas

1. **Duplicate `token_count` re-emissions**: Codex periodically re-emits the same cumulative `token_count` snapshot (with the previous `last_token_usage` still present) even though zero new tokens occurred. Skip any `token_count` whose `total_token_usage` matches `previous_totals` on all token channels (or if `total_token_usage` is absent, skip identical consecutive `last_token_usage` signatures).
2. **Forked sessions (`forked_from_id`)**: When `session_meta.payload.forked_from_id` is set, events at or before `session_meta.timestamp` reflect history copied from the parent session; anchor `previous_totals` without double-billing those copied events.
3. **Request ledger vs cumulative fallback**: When `token_usage_record` events with `response_id` are active for a turn, prefer the ledger records over redundant `token_count` summaries until the next `turn_context`.
4. **Priority / Fast tier (`thread_settings_applied`)**: Carry forward `service_tier` across events in the file (and persist in incremental parse resume state). Fast/Priority on GPT-5.4 / GPT-5.3-Codex doubles token rates (`_priority` in LiteLLM).
5. **Effort suffix aliases**: Strip `-xhigh`, `-high`, `-medium`, `-low`, `-none` when resolving exact model prices (e.g., `gpt-5.4-high` -> `gpt-5.4`) before prefix fallbacks.

## Token Semantics

- **Inclusive input**: `input_tokens` includes `cached_input_tokens` (and `cache_write_input_tokens` when present):
  - `uncached_input = max(0, input_tokens - cached_input_tokens - cache_write_input_tokens)`
- **Reasoning tokens**: Included inside `output_tokens` (tracked separately for breakdown, never added a second time).

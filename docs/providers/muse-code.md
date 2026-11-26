# Muse Code (`muse`)

- **Default Paths**:
  - Override flag: `--muse-sessions-dir`
  - Environment / XDG: `$MUSE_HOME/sessions/**/session.jsonl`, `$XDG_DATA_HOME/muse/sessions/**/session.jsonl`, `~/.local/share/muse/sessions/**/session.jsonl` (and macOS `~/Library/Application Support/muse/sessions/**/session.jsonl`)
- **Format**: Structured runtime event JSONL (`session.jsonl` per session directory).

## Schema (`runtime.session`, `payload_schema_version` `1` or `3`)

```json
{
  "schema_version": 1,
  "record_type": "event",
  "payload_type": "runtime.session",
  "payload_schema_version": 1,
  "id": "00000000-0000-4000-8000-000000000001",
  "recorded_at": 1760000000123000,
  "stream": {
    "kind": "session",
    "id": "sess_01ExampleMuse"
  },
  "payload": {
    "source_run_record_id": "run_01ExampleMuse",
    "event": {
      "kind": "model_completed",
      "model": "claude-sonnet-4-6",
      "usage": {
        "input_tokens": 2000,
        "output_tokens": 300,
        "cache_read_tokens": 1200,
        "cache_write_tokens": 200,
        "reasoning_tokens": 80
      }
    }
  }
}
```

## Parsing Rules & Gotchas

1. **Event Filter**: Require `schema_version == 1`, `record_type == "event"`, `payload_type == "runtime.session"`, `payload_schema_version` in `{1, 3}`, `stream.kind == "session"`, and `payload.event.kind` in `{"model_completed", "automated_review_completed"}`.
2. **Timestamps**: `recorded_at` is an integer timestamp in **microseconds** (`recorded_at / 1_000` milliseconds).
3. **Model Shape**: `payload.event.model` may be either a plain string (`"claude-sonnet-4-6"`) or an object (`{"model_id": "gpt-5.4"}`).
4. **Cached Key Aliases**: Cached input may appear as `cache_read_tokens`, `cached_input_tokens`, or `cached_tokens`.
5. **Copied Runtime Envelopes**: Runtime envelopes can be re-emitted across sessions with a new top-level `id`. Always prefer `payload.source_run_record_id` (falling back to `entry.id`) for deduplication.

## Token Semantics

- **Inclusive input**: `input_tokens` includes both `cache_read` and `cache_write_tokens`:
  - `cached_read = min(input_tokens, cache_read_tokens ?? cached_input_tokens ?? cached_tokens ?? 0)`
  - `cache_write = min(input_tokens - cached_read, cache_write_tokens ?? 0)`
  - `uncached_input = input_tokens - cached_read - cache_write`
- **Reasoning**: Included inside `output_tokens` (clamped with `min(output_tokens, reasoning_tokens)`).

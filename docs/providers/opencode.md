# OpenCode (`opencode`)

- **Default Discovery Roots**:
  - `$OPENCODE_DATA_DIR` (comma-separated allowed)
  - `$XDG_DATA_HOME/opencode`
  - `~/.local/share/opencode`
  - macOS legacy fallback: `~/Library/Application Support/opencode` and `~/Library/Application Support/OpenCode`
- **Formats (Dual Storage)**:
  1. **SQLite database**: `<root>/opencode.db` (queries `message` table rows `id`, `session_id`, `time_created`, `data`).
  2. **Legacy JSON files**: `<root>/storage/message/<sessionId>/<msgId>.json`.

## Schema (Message JSON Object)

```json
{
  "id": "msg_01ExampleOpenCodeId",
  "sessionID": "ses_01ExampleSessionId",
  "role": "assistant",
  "providerID": "opencode",
  "model": {
    "id": "claude-sonnet-4-6",
    "providerID": "anthropic"
  },
  "time": {
    "created": 1760000000000,
    "completed": 1760000005000
  },
  "tokens": {
    "input": 1500,
    "output": 240,
    "reasoning": 60,
    "cache": {
      "read": 8000,
      "write": 500
    }
  },
  "cost": 0.0
}
```

## Parsing Rules & Gotchas

1. **DB + Legacy Deduplication**: OpenCode 1.2+ migrated messages from individual JSON files under `storage/message/` into `opencode.db`. Both sources can coexist on disk for the same message ID; SQLite rows take precedence per `opencode:${id}`.
2. **Role filter**: Only `role == "assistant"` (or omitted `role` in legacy JSONs) counts toward billable usage.
3. **Zero Cost Semantics**: OpenCode records `"cost": 0` for *both* genuinely free gateway tiers (e.g. `opencode` / `opencode-go` models ending in `-free`) *and* unknown/subscription models. Treat `cost > 0` or `(gateway in {"opencode", "opencode-go"} && model.ends_with("-free") && cost == 0)` as provider-reported; otherwise estimate from model pricing tables.
4. **OpenCode Go Rate Prefixing**: Bare model IDs served via `providerID == "opencode-go"` resolve under LiteLLM's `opencode_go/<model>` namespace.

## Token Semantics

- **Disjoint input**: `tokens.input` excludes `tokens.cache.read` and `tokens.cache.write`.
- **Additive reasoning**: Raw `tokens.output` *excludes* `tokens.reasoning`, so total billable output tokens equal `tokens.output + tokens.reasoning` (`reasoning_output_tokens = tokens.reasoning`).

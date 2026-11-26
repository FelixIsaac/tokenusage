# Claude Code (`claude`)

- **Default Paths**: `$CLAUDE_CONFIG_DIR/projects/**/*.jsonl` (defaults to `~/.claude/projects/**/*.jsonl`) and `~/.config/claude/projects/**/*.jsonl`.
- **Format**: Append-only JSONL transcripts (one JSON object per streaming chunk, message, or user/system event). Subagent transcripts live under `<project>/<sessionId>/subagents/*.jsonl`.

## Schema & Relevant Fields

Each API assistant message is emitted one or more times (streaming progress / tool-use blocks) with `"type": "assistant"`:

```json
{
  "type": "assistant",
  "timestamp": "2026-10-01T12:00:00.000Z",
  "sessionId": "00000000-0000-4000-8000-000000000001",
  "requestId": "req_01ExampleRequestId",
  "costUSD": 0.042,
  "message": {
    "id": "msg_01ExampleMessageId",
    "model": "claude-opus-4-6",
    "usage": {
      "input_tokens": 1200,
      "cache_creation_input_tokens": 4000,
      "cache_read_input_tokens": 25000,
      "output_tokens": 350,
      "cache_creation": {
        "ephemeral_5m_input_tokens": 0,
        "ephemeral_1h_input_tokens": 4000
      },
      "speed": "standard",
      "iterations": [
        {
          "type": "advisor_message",
          "model": "claude-opus-4-6",
          "input_tokens": 800,
          "cache_creation_input_tokens": 0,
          "cache_read_input_tokens": 0,
          "output_tokens": 120
        }
      ]
    }
  }
}
```

## Parsing Rules & Gotchas

1. **Full-line scanning & late fields**: Top-level `"type":"assistant"` and `"usage":{...}` frequently appear **after byte 2,048** (e.g., when long UUID/tool metadata precedes `"type"`). Fast pre-filters must scan the full line for `"usage"`, never a fixed prefix slice.
2. **Duplicate keys & nullable arrays**: Real transcripts can include both `"sessionId"` and `"session_id"` (or `"requestId"` and `"request_id"`) on the same JSON object, and nullable `"iterations": null`, `"speed": null`, `"inference_geo": null`. Parsers must not fail with duplicate-field or null-sequence deserialization errors.
3. **Max-merge deduplication (intra-file and cross-file)**: Claude Code writes multiple rows sharing the same `message.id:requestId` across streaming updates, content blocks, subagents, or resumed/copied session transcripts. Instead of keeping first-seen, merge duplicate records using **`max()` per token channel**:
   - `input_tokens = max(...)`
   - `cache_creation_input_tokens = max(...)`
   - `cache_creation_1h_tokens = max(...)`
   - `cache_read_input_tokens = max(...)`
   - `output_tokens = max(...)`
   - `reported_cost_usd = max(...)`
4. **Placeholder models**: Skip `<synthetic>` / `synthetic` (local non-API placeholder messages) and bare family names (`opus`, `sonnet`, `haiku`).
5. **1-Hour vs 5-Minute Cache Writes**: `usage.cache_creation_input_tokens` includes *both* 5m and 1h TTL writes. Split them as:
   - `cw_1h = min(usage.cache_creation.ephemeral_1h_input_tokens, cache_creation_input_tokens)`
   - `cw_5m = cache_creation_input_tokens - cw_1h`
6. **Fast Mode & Context Variants**:
   - `usage.speed == "fast"` enables `ServiceSpeed::Fast` pricing (`provider_specific_entry.fast` in LiteLLM).
   - Bracketed suffixes like `claude-opus-4-6[1m]` strip `[...]` before rate lookup (`claude-opus-4-6`).
7. **Advisor Iterations**: `usage.iterations[]` entries with `type == "advisor_message"` and a non-synthetic `model` represent separate billable calls on the advisor model and emit their own usage record with dedupe key `${dedupeKey}:advisor:${index}` (inheriting executor `speed` and timestamp, with `reportedCostUsd = null`).
8. **Side-channel text generation**: Modern Claude Code background title/summary calls should be ignored unless part of session assistant usage (`type != "assistant"` skipped).

## Token Semantics

- **Disjoint input**: `input_tokens` counts uncached input only.
- **Reasoning / thinking**: Included inside `output_tokens`; do not add separately.

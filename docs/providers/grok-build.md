# Grok Build / xAI (`grok`)

- **Default Paths**:
  - Primary: `~/.grok/sessions/**/<sessionId>/updates.jsonl` (or `$GROK_HOME/sessions/**/updates.jsonl`)
  - Legacy fallback: `~/.grok/logs/unified.jsonl` (`shell.turn.inference_done` — only when `sessions` does not exist)
- **Important Exclusion**: Ignore sibling `chat_history.jsonl` and `events.jsonl` in session directories; only `updates.jsonl` holds canonical turn-completion billing records.
- **Format**: JSON-RPC / ACP session update JSONL (`updates.jsonl`).

## Schema (`updates.jsonl`)

```json
{
  "timestamp": 1760000000,
  "method": "_x.ai/session/update",
  "params": {
    "sessionId": "00000000-0000-4000-8000-000000000001",
    "update": {
      "sessionUpdate": "turn_completed",
      "prompt_id": "00000000-0000-4000-8000-000000000002",
      "usage": {
        "inputTokens": 60000,
        "cachedReadTokens": 45000,
        "cacheCreationTokens": 2000,
        "outputTokens": 1200,
        "reasoningTokens": 400,
        "modelCalls": 6,
        "costUsdTicks": 250000000,
        "modelUsage": {
          "grok-4.6-build": {
            "inputTokens": 60000,
            "cachedReadTokens": 45000,
            "cacheCreationTokens": 2000,
            "outputTokens": 1200,
            "reasoningTokens": 400,
            "modelCalls": 6,
            "costUsdTicks": 250000000
          }
        }
      }
    },
    "_meta": {
      "agentTimestampMs": 1760000000123
    }
  }
}
```

## Parsing Rules & Gotchas

1. **`costUsdTicks` Scale Factor**: `costUsdTicks` is denominated in **`1e-10` USD (`10_000_000_000` ticks = `$1.00`)**, **not** `1e-9`! Dividing by `1e9` overcounts provider-reported Grok costs by **10x**.
2. **Per-Model Breakdown (`modelUsage`)**: When `usage.modelUsage` is present, emit one usage record per non-zero model entry:
   - Each model entry may report its own `costUsdTicks`.
   - If a model entry omits `costUsdTicks` while top-level `usage.costUsdTicks` is present, allocate the remaining unticked top-level cost proportionally across unticked models by total tokens.
3. **Cache Creation Tokens**: Honor `cacheCreationTokens` alongside `cachedReadTokens`; both are subsets of `inputTokens`.
4. **Resumed Session `prompt_id` Collision**: Because Grok's prompt IDs reset per process launch, deduplicate using the composite tuple `${sessionId}:${prompt_id}:${timestampMs}:${model}` rather than `prompt_id` alone (do not dedupe records missing `prompt_id`).
5. **Context-Window Cliff Guard (`modelCalls > 1`)**: A single `turn_completed` record aggregates an entire agentic turn spanning `modelCalls` individual model requests (commonly 5–15+ calls). Evaluating a `>200K` or `>272K` context-window tier against the *summed* `inputTokens` causes large false cliff surcharges when `reportedCostUsd` is absent (`~$934` inflation in multi-call turns). Always compare `(prompt_tokens / max(1, modelCalls)) > threshold`.
6. **Timestamps**: Prefer `params._meta.agentTimestampMs` (ms); fall back to `timestamp` (`> 1e12` = ms, else seconds).

## Token Semantics

- **Inclusive input**: `inputTokens` includes both `cachedReadTokens` and `cacheCreationTokens`:
  - `uncached_input = max(0, inputTokens - cachedReadTokens - cacheCreationTokens)`
- **Reasoning tokens**: Included inside `outputTokens` (clamped with `min(outputTokens, reasoningTokens)`).

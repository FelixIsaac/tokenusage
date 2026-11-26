# Pi (`pi`)

- **Default Paths**:
  - Override flag: `--pi-sessions-dir`
  - Environment variables: `$PI_CODING_AGENT_SESSION_DIR`, else `$PI_CODING_AGENT_DIR/sessions`
  - Default: `~/.pi/agent/sessions/**/*.jsonl`
- **Format**: Session-header + message/usage tree JSONL.

## Relevant Entry Types

1. **`session`** — establishes the session ID for the file:
   ```json
   {
     "type": "session",
     "version": 3,
     "id": "00000000-0000-4000-8000-000000000001",
     "timestamp": "2026-10-01T12:00:00Z",
     "cwd": "/Users/dev/projects/example-app"
   }
   ```
2. **`message` (`role: "assistant"`)**:
   ```json
   {
     "type": "message",
     "id": "msg_01ExamplePi",
     "parentId": "prev_01",
     "timestamp": "2026-10-01T12:00:05Z",
     "message": {
       "role": "assistant",
       "provider": "anthropic",
       "model": "claude-sonnet-4-6",
       "responseModel": "claude-sonnet-4-6",
       "usage": {
         "input": 1000,
         "output": 200,
         "cacheRead": 5000,
         "cacheWrite": 300,
         "reasoning": 50,
         "cost": {
           "total": 0.0125
         }
       }
     }
   }
   ```
3. **Auxiliary Entries (`role: "toolResult"`, `type: "compaction"`, `type: "branch_summary"`)**:
   - Omit `provider`/`model`; attribute under `"Tools/summaries"`.
4. **Standalone `type: "usage"`**:
   - Carries top-level `provider`, `model`, and `usage`.

## Parsing Rules & Gotchas

1. **Model vs Rate Lookup**: Emitted model ID is `<provider>/<responseModel ?? model>` (or `"Tools/summaries"` for tool/summary entries). Rate lookup strips the `<provider>/` prefix when matching first-party model pricing.
2. **Fork-Safe Deduplication**: Session forks copy parent entry IDs and timestamps. Deduplicate on the tuple `("pi", entry.id, entry.timestamp, model)`.
3. **Reported Cost vs Estimation**: Use `usage.cost.total` when finite and `> 0`; Pi writes `0` when local/custom provider rates are unconfigured, so fall back to rate-table estimation whenever `cost.total <= 0`.

## Token Semantics

- **Disjoint input**: `usage.input` excludes `cacheRead` and `cacheWrite`.
- **Reasoning**: Included inside `usage.output` (clamped with `min(output, reasoning)`).

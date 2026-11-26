# Cursor (`cursor`) — Reference Specification

> **Note on `tu` Support**: Unlike file-based local agents, Cursor does **not** persist complete per-request token counts and model usage totals in local files under `~/.cursor` (`~/.cursor/chats` contains chat metadata/sqlite state rather than API billing ledgers). Because `tokenusage` enforces a zero-network, offline-first guarantee for default log scans (`tu today`, `tu daily`, `tu live`), Cursor remote API polling is intentionally excluded from `tu`'s default scan pipeline and documented here for reference.

## Remote Data Source (Cursor Account API)

Tools such as T3 Code retrieve Cursor usage via authenticated HTTP requests to Cursor's account usage endpoints (using an exchanged `CURSOR_AUTH_TOKEN` / `CURSOR_API_KEY` or stored Cursor credentials):

- **Pagination**: `POST` filtered-usage events with `{ page, pageSize, startDate, endDate, [teamId, userId] }` (`pageSize`, boundary overlap deduplication, and `total` count validation).
- **Per-Event Fields**:
  - `timestamp` (epoch ms as number or string)
  - `model` (string)
  - `conversationId` (`sessionId`)
  - `tokenUsage` / `usage`:
    - `inputTokens` (disjoint uncached input)
    - `cacheReadTokens`
    - `cacheWriteTokens`
    - `outputTokens`
    - `totalCents` -> `reportedCostUsd = totalCents / 100`

## Model Normalization (`cursorRateModel`)

Cursor model IDs often include Cursor-specific prefixes and effort/speed suffixes (`cursor-`, `-thinking`, `-none`, `-minimal`, `-low`, `-medium`, `-high`, `-xhigh`, `-max`, `-fast`):

```text
cursor-grok-4-fast           -> xai/grok-4
cursor-claude-sonnet-4-high  -> claude-sonnet-4
```

- First strip `^cursor-`, then trailing `(?:-thinking)?(?:-(?:none|minimal|low|medium|high|xhigh|max))?(?:-fast)?$`.
- If the normalized name starts with `grok-`, map to `xai/<model>` for LiteLLM lookup.

## Deduplication

Because Cursor account usage pages lack a unique server-generated event ID and overlapping page boundaries can duplicate adjacent events:
1. Canonicalize event payload `(teamId?, timestampMs, model, sessionId, totals, reportedCostUsd)` with SHA-256.
2. Track KMP-style suffix/prefix boundary overlap between consecutive pages reconciled against the server's total event count.
3. Append per-key occurrence indices (`0, 1, 2, ...`) so genuine identical calls within the same millisecond are retained while page overlaps are removed.

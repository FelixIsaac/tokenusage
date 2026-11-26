# Antigravity & Gemini CLI (`gemini` / `antigravity`)

- **Default Paths**:
  - Antigravity Protobuf sessions: `$ANTIGRAVITY_DATA_DIR/conversations/*.pb` or `~/.gemini/antigravity*/conversations/*.pb` (and SQLite `state.vscdb` model metadata where present)
  - Gemini CLI logs: `~/.gemini/tmp/**/chats/*.jsonl`
- **Pruning Note**: Antigravity `brain/`, `.system_generated/`, `skills/`, `browser_recordings/`, and `scratch/` directories do not contain billing ledgers; pruning them during directory traversal dramatically lowers `stat` overhead. Also avoid walking both `~/.gemini` and its subdirectories redundantly.

## Formats

### 1. Antigravity Conversation Protobuf (`.pb`)
- Binary protobuf storing conversation turns and nested generator event metadata:
  - Nested timestamp fields (`seconds` field `#1`, `nanos` field `#2`).
  - Token counters for `input_tokens`, `cache_read_input_tokens`, and `output_tokens` (disjoint).
  - Model attribution by string name or numeric `MODEL_IDS` enum mapping (e.g. `246` -> `gemini-2.5-pro`, `1026` -> `claude-opus-4-6`, `1402` -> `claude-opus-5-5`, `1525` -> `gemini-4-hydrogen`, `1532` -> `gemini-4-neon`, etc.).

### 2. Gemini CLI JSONL (`chats/*.jsonl`)
```json
{
  "type": "gemini",
  "timestamp": "2026-10-01T12:00:00Z",
  "model": "gemini-2.5-pro",
  "tokens": {
    "input": 2500,
    "cached": 18000,
    "output": 420,
    "thoughts": 190
  }
}
```

## Parsing Rules & Normalization

1. **Effort & Routing Variant Stripping**: Antigravity tags model names with effort or routing suffixes such as `-high`, `-medium`, `-low`, `-extra-low`, `-tiered`, `-control`, `-thinking`, `-orchestrator`, `-trial`, `-n`, `-r`. Normalize to the base model ID when looking up pricing rates (after exact match fails).
2. **Legacy Orchestrator / Preview Aliases**: Map `gemini-3.1-pro` -> `gemini-3.1-pro-preview`, `gemini-3-pro` -> `gemini-3-pro-preview`, `gemini-3-flash` -> `gemini-3-flash-preview`.
3. **Trial / Internal Unpriced Models**: Trial/internal identifiers without published rates (e.g., internal `*-trial` or `-tiered` variants absent from LiteLLM/OpenRouter) remain unpriced (`$0.00`) unless mapped by an override or pricing file.

## Token Semantics

- **Disjoint input**: `tokens.input` is uncached input; `tokens.cached` is cache read.
- **Gemini CLI thoughts**: `tokens.thoughts` is tracked as `reasoning_output_tokens`.

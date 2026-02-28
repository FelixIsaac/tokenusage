#!/usr/bin/env python3
from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEMO_ROOT = ROOT / "examples" / "demo" / "data"


def ensure(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def write_jsonl(path: Path, rows: list[dict]) -> None:
    ensure(path)
    with path.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")


def make_claude_rows(project: str, model: str, start_day: datetime) -> list[dict]:
    rows: list[dict] = []
    for i in range(20):
        day = start_day + timedelta(days=i)
        # smooth trend with a few spikes to make charts readable
        base = 9000 + i * 650
        spike = 1 if i in {3, 11, 18} else 0
        input_tokens = base + spike * 18000
        cache_create = 800 + (i % 5) * 120
        cache_read = 120_000 + i * 36_000 + spike * 90_000
        output_tokens = 3200 + i * 300 + spike * 2200

        rows.append(
            {
                "timestamp": (day + timedelta(hours=2)).isoformat().replace("+00:00", "Z"),
                "model": model,
                "usage": {
                    "input_tokens": input_tokens,
                    "cache_creation_input_tokens": cache_create,
                    "cache_read_input_tokens": cache_read,
                    "output_tokens": output_tokens,
                },
                "project": project,
            }
        )
    return rows


def make_codex_rows(model: str, start_day: datetime, step: int) -> list[dict]:
    rows: list[dict] = []
    for i in range(20):
        day = start_day + timedelta(days=i)
        base = 14_000 + i * 900
        spike = 1 if i in {4, 12, 19} else 0
        total = base + spike * 24_000
        cached = int(total * (0.68 + (i % 3) * 0.06))
        input_tokens = total + cached
        output_tokens = 3600 + i * 360 + spike * 1500

        rows.append(
            {
                "timestamp": (day + timedelta(hours=6)).isoformat().replace("+00:00", "Z"),
                "type": "turn_context",
                "payload": {"model": model},
            }
        )
        rows.append(
            {
                "timestamp": (day + timedelta(hours=6, minutes=step)).isoformat().replace(
                    "+00:00", "Z"
                ),
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "last_token_usage": {
                            "input_tokens": input_tokens,
                            "cached_input_tokens": cached,
                            "output_tokens": output_tokens,
                            "reasoning_output_tokens": int(output_tokens * 0.15),
                            "total_tokens": input_tokens + output_tokens,
                        }
                    },
                },
            }
        )
    return rows


def main() -> None:
    start = datetime(2026, 2, 9, tzinfo=timezone.utc)

    claude_alpha = make_claude_rows("alpha-project", "claude-opus-4-6", start)
    claude_beta = make_claude_rows("nebula-labs", "claude-haiku-4-5-20251001", start)

    codex_main = make_codex_rows("gpt-5.3-codex", start, 12)
    codex_alt = make_codex_rows("gpt-5.3-codex", start, 26)

    write_jsonl(DEMO_ROOT / "claude/projects/alpha-project/session-main.jsonl", claude_alpha)
    write_jsonl(DEMO_ROOT / "claude/projects/nebula-labs/session-research.jsonl", claude_beta)

    write_jsonl(DEMO_ROOT / "codex/sessions/studio-app/session-main.jsonl", codex_main)
    write_jsonl(DEMO_ROOT / "codex/sessions/infra-tools/session-infra.jsonl", codex_alt)

    print("demo data generated at", DEMO_ROOT)


if __name__ == "__main__":
    main()

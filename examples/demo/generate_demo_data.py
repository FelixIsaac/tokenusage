#!/usr/bin/env python3
from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEMO_ROOT = ROOT / "examples" / "demo" / "data"
DEMO_LOCAL_UTC_OFFSET = 8
HOUR_WEIGHTS = [
    0.08,
    0.10,
    0.12,
    0.11,
    0.13,
    0.15,
    0.17,
    0.20,
    0.23,
    0.26,
    0.30,
    0.34,
    0.40,
    0.46,
    0.54,
    0.62,
    0.57,
    0.52,
    0.47,
    0.43,
    0.39,
    0.34,
    0.28,
    0.22,
]


def ensure(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def write_jsonl(path: Path, rows: list[dict]) -> None:
    ensure(path)
    with path.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")


def local_hour_to_utc(base_day_utc: datetime, local_hour: int) -> datetime:
    # Map local-hour buckets (Asia/Shanghai in demo config) back to UTC timestamp,
    # so daily(hourly) charts on the demo day fill 00-23 without spilling to neighbors.
    return base_day_utc + timedelta(hours=local_hour - DEMO_LOCAL_UTC_OFFSET)


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

        # For the last demo day, spread usage across all 24 hours so `tu img day`
        # generates a meaningful hourly chart (instead of one spike + many zeros).
        if i == 19:
            for hour, weight in enumerate(HOUR_WEIGHTS):
                rows.append(
                    {
                        "timestamp": local_hour_to_utc(day, hour)
                        .isoformat()
                        .replace("+00:00", "Z"),
                        "model": model,
                        "usage": {
                            "input_tokens": max(120, int(input_tokens * weight * 0.22)),
                            "cache_creation_input_tokens": max(
                                30, int(cache_create * (0.75 + hour * 0.01))
                            ),
                            "cache_read_input_tokens": max(
                                2000, int(cache_read * weight * 0.12)
                            ),
                            "output_tokens": max(80, int(output_tokens * weight * 0.28)),
                        },
                        "project": project,
                    }
                )
        else:
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

        if i == 19:
            for hour, weight in enumerate(HOUR_WEIGHTS):
                hour_input = max(180, int(input_tokens * weight * 0.20))
                hour_cached = max(80, int(cached * weight * 0.22))
                hour_output = max(90, int(output_tokens * weight * 0.32))
                rows.append(
                    {
                        "timestamp": local_hour_to_utc(day, hour)
                        .isoformat()
                        .replace("+00:00", "Z"),
                        "type": "turn_context",
                        "payload": {"model": model},
                    }
                )
                rows.append(
                    {
                        "timestamp": local_hour_to_utc(day, hour)
                        .replace(minute=step)
                        .isoformat()
                        .replace("+00:00", "Z"),
                        "type": "event_msg",
                        "payload": {
                            "type": "token_count",
                            "info": {
                                "last_token_usage": {
                                    "input_tokens": hour_input,
                                    "cached_input_tokens": hour_cached,
                                    "output_tokens": hour_output,
                                    "reasoning_output_tokens": int(hour_output * 0.15),
                                    "total_tokens": hour_input + hour_output,
                                }
                            },
                        },
                    }
                )
        else:
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

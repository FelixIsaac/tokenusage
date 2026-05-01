# Security Audit Notes (2026-04-30)

Command run:

```bash
cargo audit
```

## Summary

- No critical remote-code-execution advisories were reported for the parsing/runtime core.
- Current findings are dependency warnings in transitive UI/terminal stacks and ecosystem advisories that require upstream crate updates.

## Findings

1. `RUSTSEC-2024-0384` (`instant` unmaintained)
- Path: `iced_*` / GUI stack
- Risk to core CLI parser path: low
- Mitigation: track upstream `iced` dependency updates; avoid enabling GUI in minimal deployments.

2. `RUSTSEC-2024-0436` (`paste` unmaintained)
- Path: `ratatui` and `wgpu-hal` tree
- Risk to core CLI parser path: low
- Mitigation: update `ratatui`/render stack when upstream drops `paste`.

3. `RUSTSEC-2026-0002` (`lru` unsound `IterMut`)
- Path: `ratatui` and `iced_glyphon`
- Risk: medium for affected code paths if unsafe iterator pattern is exercised.
- Mitigation: update to patched `lru` once available through upstream crates.

4. `RUSTSEC-2026-0097` (`rand` logger unsoundness)
- Path: transitive (`phf`, `quinn-proto`)
- Risk: low-to-medium depending on custom logger interactions.
- Mitigation: keep lockfile updated; retest after upstream patches.

5. Yanked crate warning (`drm 0.14.2`)
- Path: graphics stack (`softbuffer`, `iced_tiny_skia`)
- Risk: maintenance/supply-chain hygiene, not immediate exploit signal.
- Mitigation: refresh render stack deps on next UI upgrade cycle.

## Accepted Exceptions (Current)

Accepted for now due to transitive-only status and lack of direct exploit path in default CLI parse/report workflow:

- `RUSTSEC-2024-0384`
- `RUSTSEC-2024-0436`
- `RUSTSEC-2026-0002`
- `RUSTSEC-2026-0097`

## Operational Hardening Applied in This Refactor

- Added max JSON payload size guards for line/file parsing paths to reduce malformed/local-log abuse impact.
- OpenCode merge dedupe is deterministic to prevent accidental event double counting.
- Doctor diagnostics now expose discovered vs retained counts to detect anomalous ingestion behavior.

## Follow-up

- Add CI job to run `cargo audit` and fail only on high/critical or direct-runtime advisories.
- Split optional GUI dependencies behind stricter features for hardened server-only builds.

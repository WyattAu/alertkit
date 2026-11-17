# PATTERN_ADOPTION

Relationship of alertkit to the WyattAu pattern canon
(WyattAu/pattern-standards). Statuses and evidence format:
`references/adoption-format.md`. Validated by
`scripts/adoption-check.py --file PATTERN_ADOPTION.md`.

| Pattern | Status | Evidence |
|---|---|---|
| T1 | adopted | src/rule.rs:6 (rules deserialize straight from TOML into typed fields), src/rule.rs:33 (conditions are a tagged enum; there is no post-parse validation pass) |
| T13 | adopted | src/engine.rs:44 (evaluation is rule matching plus cooldown state, no IO); the IO shell is src/telegram.rs:34 (HTTP sink, failures logged and never propagated) |
| P51 | partial | Cargo.toml:13 (the fetch-kit circuit-breaker feature backs the HTTP client built at src/telegram.rs:23); the direct breaker dependency declared at Cargo.toml:14 is unreferenced in src |
| P27 | refused | R8 (Rule and Alert carry no criticality field, src/rule.rs:7 and src/engine.rs:54; the platform runs a single tier, and three honest tiers or none means none) |
| P45 | n/a | no SLO stream exists to compute burn rates against; static thresholds plus per-entity cooldowns are the entire mechanism, src/engine.rs:88 |

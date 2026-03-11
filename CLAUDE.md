# Project Rules

- Never use `serde_json::Value`. Always define typed structs with `Serialize`/`Deserialize`. This applies to both request bodies and response parsing.
- No dead code, unless you are about to use it (e.g. building incrementally across phases). If it's not used in production and not part of the current implementation plan, delete it. No `#[allow(dead_code)]`, no `#[cfg(test)]`-only methods.
- Every `#[allow(clippy::...)]` MUST have a justification comment explaining why the lint is suppressed.
- You MUST complete ALL phases of EVERY plan. Do NOT stop after a phase to summarize, ask for confirmation, or check in. Keep working continuously until every single phase is done. No exceptions.

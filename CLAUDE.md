# Project Rules

- Never use `serde_json::Value`. Always define typed structs with `Serialize`/`Deserialize`. This applies to both request bodies and response parsing.
- No dead code. If it's not used in production, delete it. No `#[allow(dead_code)]`, no `#[cfg(test)]`-only methods.
- Every `#[allow(clippy::...)]` MUST have a justification comment explaining why the lint is suppressed.

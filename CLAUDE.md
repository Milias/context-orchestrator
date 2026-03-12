# Project Rules

- Never use `serde_json::Value`. Always define typed structs with `Serialize`/`Deserialize`. This applies to both request bodies and response parsing.
- No dead code, unless you are about to use it (e.g. building incrementally across phases). If it's not used in production and not part of the current implementation plan, delete it. No `#[allow(dead_code)]`, no `#[cfg(test)]`-only methods.
- Every `#[allow(clippy::...)]` MUST have a justification comment explaining why the lint is suppressed.
- NEVER use `#[allow(clippy::too_many_lines)]` or `#[allow(clippy::too_many_arguments)]` unless the user gives explicit permission. Instead, refactor: extract helpers, create config structs using builder pattern (with a crate like `derive_builder` or `bon` as needed to automatically convert struct fields into builder methods), or split into modules.
- Files MUST be at most 400 lines. If a file grows beyond that, split it into modules.
- Tests MUST be in a separate file: `tests.rs` if the source is a module directory, or `<name>_tests.rs` if not.
- Every test MUST answer the question "what bug does this test catch?". If you can't articulate a specific bug or invariant violation the test would detect, don't write it. No low-effort tests that just call a function and assert "it doesn't crash" or "the output is non-empty".
- You MUST complete ALL phases of EVERY plan. Do NOT stop after a phase to summarize, ask for confirmation, or check in. Keep working continuously until every single phase is done. No exceptions.
- Do NOT directly repeat this rules in the plan UNLESS you are anticipating work related to them.

use super::execute;
use super::execute_tool;
use crate::graph::tool_types::ToolCallArguments;

// ── ConfigKey parsing tests ─────────────────────────────────────────

/// Bug: `ConfigKey::from_str` accepts garbage input instead of returning `Err`.
#[test]
fn test_config_key_from_str_rejects_invalid() {
    assert!(
        "invalid_key".parse::<execute::ConfigKey>().is_err(),
        "unknown key should be rejected"
    );
}

/// Bug: `ConfigKey::from_str` / `as_str` round-trip broken for a variant —
/// downstream code uses the wrong string and silently no-ops.
#[test]
fn test_config_key_round_trip_all_variants() {
    let variants = [
        ("max_tokens", execute::ConfigKey::MaxTokens),
        ("max_context_tokens", execute::ConfigKey::MaxContextTokens),
        ("model", execute::ConfigKey::Model),
        (
            "max_tool_loop_iterations",
            execute::ConfigKey::MaxToolLoopIterations,
        ),
    ];
    for (s, expected) in variants {
        let parsed: execute::ConfigKey = s.parse().unwrap();
        assert_eq!(parsed, expected, "parse mismatch for {s}");
        assert_eq!(parsed.to_string(), s, "display mismatch for {s}");
    }
}

// ── set tool validation tests ───────────────────────────────────────

/// Bug: `validate_set_value` accepts 0 for `max_tokens` — downstream
/// API call fails with an opaque error.
#[tokio::test]
async fn test_set_max_tokens_rejects_zero() {
    let args = ToolCallArguments::Set {
        key: "max_tokens".to_string(),
        value: "0".to_string(),
    };
    let result = execute_tool(&args, None).await;
    assert!(result.is_error, "max_tokens=0 should be rejected");
}

/// Bug: `validate_set_value` accepts 200000 for `max_tokens` —
/// exceeds API limit of 128000.
#[tokio::test]
async fn test_set_max_tokens_rejects_over_limit() {
    let args = ToolCallArguments::Set {
        key: "max_tokens".to_string(),
        value: "200000".to_string(),
    };
    let result = execute_tool(&args, None).await;
    assert!(result.is_error, "max_tokens=200000 should be rejected");
}

/// Bug: `execute_set` with empty value returns success instead of error.
#[tokio::test]
async fn test_set_empty_value_returns_error() {
    let args = ToolCallArguments::Set {
        key: "max_tokens".to_string(),
        value: String::new(),
    };
    let result = execute_tool(&args, None).await;
    assert!(result.is_error, "empty value should be rejected");
}

/// Bug: valid `set` command returns `is_error=true` or wrong message.
#[tokio::test]
async fn test_set_valid_value_succeeds() {
    let args = ToolCallArguments::Set {
        key: "max_tokens".to_string(),
        value: "8192".to_string(),
    };
    let result = execute_tool(&args, None).await;
    assert!(!result.is_error, "valid set should succeed");
    assert!(result.content.text_content().contains("8192"));
}

/// Bug: `apply_config_set` does not mutate the config — session change
/// is silently lost.
#[test]
fn test_apply_config_set_mutates_config() {
    let mut config = crate::config::AppConfig {
        anthropic_base_url: String::new(),
        anthropic_auth_token: None,
        anthropic_api_key: None,
        anthropic_model: "old-model".to_string(),
        max_tokens: 100,
        max_context_tokens: 1000,
        system_prompt: String::new(),
        max_tool_loop_iterations: 5,
        max_concurrent_agents: 3,
    };

    execute::apply_config_set(&mut config, execute::ConfigKey::MaxTokens, "4096");
    assert_eq!(config.max_tokens, 4096);

    execute::apply_config_set(&mut config, execute::ConfigKey::Model, "claude-opus");
    assert_eq!(config.anthropic_model, "claude-opus");

    execute::apply_config_set(&mut config, execute::ConfigKey::MaxToolLoopIterations, "20");
    assert_eq!(config.max_tool_loop_iterations, 20);

    execute::apply_config_set(&mut config, execute::ConfigKey::MaxContextTokens, "50000");
    assert_eq!(config.max_context_tokens, 50000);
}

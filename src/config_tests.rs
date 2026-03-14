use super::{AppConfig, ContextSelectionMode};

/// Bug: `api_key()` returns `Ok` when neither env var is set,
/// causing downstream HTTP requests to fail with an opaque 401.
#[test]
fn test_api_key_missing_both_returns_err() {
    let config = AppConfig {
        anthropic_auth_token: None,
        anthropic_api_key: None,
        anthropic_base_url: String::new(),
        anthropic_model: String::new(),
        max_tokens: 0,
        max_context_tokens: 0,
        system_prompt: String::new(),
        max_tool_loop_iterations: 0,
        max_concurrent_agents: 3,
        context_selection: super::ContextSelectionMode::Heuristic,
        context_selector_model: None,
    };
    assert!(config.api_key().is_err());
}

/// Bug: `api_key()` returns `anthropic_api_key` when both are set,
/// ignoring the preferred `anthropic_auth_token`. This matters when
/// the auth token has different scopes than the API key.
#[test]
fn test_api_key_prefers_auth_token_over_api_key() {
    let config = AppConfig {
        anthropic_auth_token: Some("token-preferred".to_string()),
        anthropic_api_key: Some("key-fallback".to_string()),
        anthropic_base_url: String::new(),
        anthropic_model: String::new(),
        max_tokens: 0,
        max_context_tokens: 0,
        system_prompt: String::new(),
        max_tool_loop_iterations: 0,
        max_concurrent_agents: 3,
        context_selection: super::ContextSelectionMode::Heuristic,
        context_selector_model: None,
    };
    assert_eq!(config.api_key().unwrap(), "token-preferred");
}

/// Bug: `api_key()` fails when only `anthropic_api_key` is set
/// (the fallback path is broken).
#[test]
fn test_api_key_falls_back_to_api_key() {
    let config = AppConfig {
        anthropic_auth_token: None,
        anthropic_api_key: Some("key-only".to_string()),
        anthropic_base_url: String::new(),
        anthropic_model: String::new(),
        max_tokens: 0,
        max_context_tokens: 0,
        system_prompt: String::new(),
        max_tool_loop_iterations: 0,
        max_concurrent_agents: 3,
        context_selection: super::ContextSelectionMode::Heuristic,
        context_selector_model: None,
    };
    assert_eq!(config.api_key().unwrap(), "key-only");
}

/// Bug: `"llm_guided"` rejected by serde deserialization — the LLM-guided
/// context selection feature is silently disabled, falling back to heuristic.
#[test]
fn context_selection_mode_deserializes() {
    let heuristic: ContextSelectionMode = serde_json::from_str(r#""heuristic""#).unwrap();
    assert_eq!(heuristic, ContextSelectionMode::Heuristic);

    let llm: ContextSelectionMode = serde_json::from_str(r#""llm_guided""#).unwrap();
    assert_eq!(llm, ContextSelectionMode::LlmGuided);
}

/// Bug: default config values accidentally changed — agents use wrong
/// token limits, model names, or iteration caps.
#[test]
fn default_config_values_correct() {
    // Construct via serde defaults by deserializing an empty env.
    // We can't call load() (reads real env), so verify the default functions.
    assert_eq!(super::default_max_tokens(), 16384);
    assert_eq!(super::default_max_context_tokens(), 180_000);
    assert_eq!(super::default_model(), "claude-sonnet-4-6");
    assert_eq!(super::default_max_tool_loop_iterations(), 10);
    assert_eq!(super::default_max_concurrent_agents(), 3);
}

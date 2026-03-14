use super::AppConfig;

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
        max_concurrent_agents: 0,
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
        max_concurrent_agents: 0,
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
        max_concurrent_agents: 0,
    };
    assert_eq!(config.api_key().unwrap(), "key-only");
}

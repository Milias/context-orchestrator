use figment::{providers::Env, Figment};
use serde::Deserialize;

fn default_base_url() -> String {
    "https://api.anthropic.com".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_max_tokens() -> u32 {
    16384
}

fn default_max_context_tokens() -> u32 {
    180_000
}

fn default_system_prompt() -> String {
    "You are a helpful assistant.".to_string()
}

fn default_max_tool_loop_iterations() -> usize {
    10
}

fn default_max_concurrent_agents() -> usize {
    3
}

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_base_url")]
    pub anthropic_base_url: String,
    pub anthropic_auth_token: Option<String>,
    pub anthropic_api_key: Option<String>,
    #[serde(default = "default_model")]
    pub anthropic_model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u32,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_max_tool_loop_iterations")]
    pub max_tool_loop_iterations: usize,
    /// Maximum number of concurrent ephemeral agents.
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,
    /// Context selection mode: "heuristic" (default, zero API cost) or
    /// "llm_guided" (adds a meta-LLM call to refine candidate selection).
    #[serde(default = "default_context_selection")]
    pub context_selection: String,
    /// Model for the LLM refinement layer in context selection.
    /// Defaults to the main model. Can use a cheaper model (e.g., Haiku).
    pub context_selector_model: Option<String>,
}

fn default_context_selection() -> String {
    "heuristic".to_string()
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        Figment::new()
            .merge(Env::raw())
            .extract()
            .map_err(|e| anyhow::anyhow!("Configuration error: {e}"))
    }

    pub fn api_key(&self) -> anyhow::Result<String> {
        self.anthropic_auth_token
            .clone()
            .or_else(|| self.anthropic_api_key.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("Neither ANTHROPIC_AUTH_TOKEN nor ANTHROPIC_API_KEY set")
            })
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

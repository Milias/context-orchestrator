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

/// How the context scoring pipeline selects nodes for the context window.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSelectionMode {
    /// Deterministic edge-weighted scoring only. Zero API cost.
    #[default]
    Heuristic,
    /// Deterministic scoring followed by a meta-LLM refinement call.
    LlmGuided,
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
    /// How to select nodes for the context window (heuristic or LLM-guided).
    #[serde(default)]
    pub context_selection: ContextSelectionMode,
    /// Model override for the meta-LLM selector (only used when `context_selection = "llm_guided"`).
    /// Falls back to `anthropic_model` when `None`.
    pub context_selector_model: Option<String>,
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

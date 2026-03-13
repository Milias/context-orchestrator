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

fn default_background_model() -> String {
    "claude-opus-4-6".to_string()
}

fn default_background_max_tokens() -> u32 {
    1024
}

fn default_background_max_concurrent() -> usize {
    2
}

fn default_max_tool_loop_iterations() -> usize {
    10
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
    #[serde(default = "default_background_model")]
    pub background_model: String,
    #[serde(default = "default_background_max_tokens")]
    pub background_max_tokens: u32,
    #[serde(default = "default_background_max_concurrent")]
    pub background_max_concurrent: usize,
    #[serde(default = "default_max_tool_loop_iterations")]
    pub max_tool_loop_iterations: usize,
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

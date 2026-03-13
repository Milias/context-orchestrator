mod app;
mod config;
mod graph;
mod llm;
mod migration;
mod persistence;
mod tasks;
mod tool_executor;
mod tools;
mod tui;

use app::App;
use chrono::Utc;
use clap::{Parser, Subcommand};
use config::AppConfig;
use graph::ConversationGraph;
use llm::anthropic::AnthropicProvider;
use persistence::ConversationMetadata;
use std::io;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "context-manager",
    version,
    about = "Graph-based context manager for LLM conversations"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new conversation
    New {
        /// Name for the new conversation
        #[arg(default_value = "New Conversation")]
        name: String,
    },
    /// List all conversations
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load()?;

    // Handle list subcommand (no API key needed)
    if let Some(Command::List) = &cli.command {
        let conversations = persistence::list_conversations().unwrap_or_default();
        if conversations.is_empty() {
            println!("No conversations found.");
        } else {
            for conv in &conversations {
                println!("{} - {} ({})", conv.id, conv.name, conv.last_modified);
            }
        }
        return Ok(());
    }

    // API key required from here on
    config.api_key()?;
    let provider = AnthropicProvider::from_config(&config)?;

    let (metadata, graph) = if let Some(Command::New { name }) = &cli.command {
        let id = Uuid::new_v4().to_string();
        let graph = ConversationGraph::new(&config.system_prompt);
        let metadata = ConversationMetadata {
            id: id.clone(),
            name: name.clone(),
            created_at: Utc::now(),
            last_modified: Utc::now(),
        };
        persistence::save_conversation(&id, &metadata, &graph)?;
        (metadata, graph)
    } else {
        let conversations = persistence::list_conversations().unwrap_or_default();
        if conversations.is_empty() {
            let id = Uuid::new_v4().to_string();
            let graph = ConversationGraph::new(&config.system_prompt);
            let metadata = ConversationMetadata {
                id: id.clone(),
                name: "Default Conversation".to_string(),
                created_at: Utc::now(),
                last_modified: Utc::now(),
            };
            persistence::save_conversation(&id, &metadata, &graph)?;
            (metadata, graph)
        } else {
            let latest = &conversations[0];
            persistence::load_conversation(&latest.id)?
        }
    };

    // Panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags,
            crossterm::terminal::LeaveAlternateScreen
        );
        original_hook(panic_info);
    }));

    let app = App::new(config, graph, metadata, Arc::new(provider));
    app.run().await
}

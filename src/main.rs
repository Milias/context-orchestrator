mod app;
mod graph;
mod llm;
mod persistence;
mod tui;

use app::App;
use chrono::Utc;
use graph::ConversationGraph;
use llm::anthropic::AnthropicProvider;
use persistence::ConversationMetadata;
use std::io;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Handle --list
    if args.iter().any(|a| a == "--list") {
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

    // Check for API key early
    if std::env::var("ANTHROPIC_AUTH_TOKEN").is_err() && std::env::var("ANTHROPIC_API_KEY").is_err()
    {
        anyhow::bail!("Neither ANTHROPIC_AUTH_TOKEN nor ANTHROPIC_API_KEY environment variable set");
    }

    let provider = AnthropicProvider::new()?;

    // Handle --new "name"
    let (metadata, graph) = if let Some(pos) = args.iter().position(|a| a == "--new") {
        let name = args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "New Conversation".to_string());
        let id = Uuid::new_v4().to_string();
        let graph = ConversationGraph::new("You are a helpful assistant.");
        let metadata = ConversationMetadata {
            id: id.clone(),
            name,
            created_at: Utc::now(),
            last_modified: Utc::now(),
        };
        persistence::save_conversation(&id, &metadata, &graph)?;
        (metadata, graph)
    } else {
        let conversations = persistence::list_conversations().unwrap_or_default();
        if conversations.is_empty() {
            let id = Uuid::new_v4().to_string();
            let graph = ConversationGraph::new("You are a helpful assistant.");
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
        let _ = crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let app = App::new(graph, metadata, Box::new(provider));
    app.run().await
}

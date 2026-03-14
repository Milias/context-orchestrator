use super::{parse_selection, render_summaries};
use crate::app::context::scoring::{ScoredCandidate, SelectionTier};
use crate::graph::{ConversationGraph, Node, Role};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// Helper: build a scored candidate with a matching graph node.
fn candidate_in_graph(
    graph: &mut ConversationGraph,
    score: f64,
    tier: SelectionTier,
    content: &str,
) -> ScoredCandidate {
    let id = Uuid::new_v4();
    let node = Node::Message {
        id,
        role: Role::User,
        content: content.to_string(),
        created_at: Utc::now(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        stop_reason: None,
    };
    graph.add_node(node);
    ScoredCandidate {
        node_id: id,
        score,
        tier,
    }
}

/// Bug: valid JSON response with correct short IDs incorrectly falls back,
/// so the LLM's selection is ignored and the agent gets all candidates.
#[test]
fn parse_selection_extracts_valid_ids() {
    let mut graph = ConversationGraph::new("sys");
    let c1 = candidate_in_graph(&mut graph, 0.9, SelectionTier::Essential, "hello");
    let c2 = candidate_in_graph(&mut graph, 0.5, SelectionTier::Important, "world");
    let candidates = vec![c1.clone(), c2];

    let (_, id_map) = render_summaries(&graph, &candidates);
    let short_id = &c1.node_id.to_string()[..8];
    let response = format!(r#"{{"selected": ["{short_id}"], "reasoning": "most relevant"}}"#);

    let result = parse_selection(&response, &id_map, &candidates);
    assert!(
        !result.is_fallback,
        "should use LLM selection, not fallback"
    );
    assert_eq!(result.selected_ids, vec![c1.node_id]);
}

/// Bug: response with no JSON braces causes panic (string slice out of bounds)
/// instead of falling back to heuristic.
#[test]
fn parse_selection_no_json_falls_back() {
    let candidates = vec![ScoredCandidate {
        node_id: Uuid::new_v4(),
        score: 0.8,
        tier: SelectionTier::Essential,
    }];
    let id_map = HashMap::new();

    let result = parse_selection("no json here at all", &id_map, &candidates);
    assert!(result.is_fallback);
    assert_eq!(result.selected_ids.len(), 1);
}

/// Bug: LLM returns `{"selected": []}` — agent gets zero context nodes,
/// producing an empty prompt that wastes an API call.
#[test]
fn parse_selection_empty_selected_falls_back() {
    let candidates = vec![ScoredCandidate {
        node_id: Uuid::new_v4(),
        score: 0.8,
        tier: SelectionTier::Essential,
    }];
    let id_map = HashMap::new();

    let result = parse_selection(
        r#"{"selected": [], "reasoning": "none"}"#,
        &id_map,
        &candidates,
    );
    assert!(
        result.is_fallback,
        "empty selection should trigger fallback"
    );
}

/// Bug: hallucinated short IDs pass through to `selected_ids`, referencing
/// nodes that don't exist in the graph — downstream code panics on lookup.
#[test]
fn parse_selection_unknown_ids_filtered() {
    let mut graph = ConversationGraph::new("sys");
    let c = candidate_in_graph(&mut graph, 0.9, SelectionTier::Essential, "real");
    let candidates = vec![c.clone()];
    let (_, id_map) = render_summaries(&graph, &candidates);
    let real_short = &c.node_id.to_string()[..8];

    let response =
        format!(r#"{{"selected": ["{real_short}", "deadbeef"], "reasoning": "picked"}}"#);
    let result = parse_selection(&response, &id_map, &candidates);
    assert!(!result.is_fallback);
    assert_eq!(
        result.selected_ids,
        vec![c.node_id],
        "hallucinated ID should be silently dropped"
    );
}

/// Bug: short ID → full UUID mapping is wrong, so all LLM selections fail
/// because none of the returned IDs resolve.
#[test]
fn render_summaries_maps_short_ids() {
    let mut graph = ConversationGraph::new("sys");
    let c = candidate_in_graph(&mut graph, 0.9, SelectionTier::Essential, "test content");
    let candidates = vec![c.clone()];

    let (text, id_map) = render_summaries(&graph, &candidates);
    let short_id = &c.node_id.to_string()[..8];

    assert!(
        id_map.contains_key(short_id),
        "short ID {short_id} must be in id_map"
    );
    assert_eq!(id_map[short_id], c.node_id);
    assert!(
        text.contains(short_id),
        "summary text must include the short ID"
    );
}

/// Bug: stream error during meta-LLM call doesn't trigger fallback,
/// leaving `refine()` with an empty/partial response that yields zero nodes.
#[tokio::test]
async fn refine_stream_error_falls_back() {
    use crate::llm::mock::MockLlmProvider;
    use crate::llm::StreamChunk;
    use std::sync::Arc;

    let provider: Arc<dyn crate::llm::LlmProvider> = Arc::new(
        MockLlmProvider::with_token_count(100)
            .with_chunks(vec![StreamChunk::Error("connection reset".to_string())]),
    );

    let mut graph = ConversationGraph::new("sys");
    let c = candidate_in_graph(&mut graph, 0.9, SelectionTier::Essential, "node");
    let candidates = vec![c.clone()];

    let result = super::refine(&provider, "test-model", &graph, &candidates, "do stuff").await;
    assert!(result.is_fallback, "stream error should trigger fallback");
    assert_eq!(result.selected_ids, vec![c.node_id]);
}

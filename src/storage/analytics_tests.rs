//! Tests for the analytics token store.

use crate::storage::schema::{TokenDirection, TokenEvent, TokenTotals};
use crate::storage::TokenStore;
use tempfile::NamedTempFile;

/// Catches: wrong column mapping in INSERT or SUM, direction string
/// mismatch between write and read paths. Records events across two
/// conversations and verifies `lifetime_totals` returns the cross-
/// conversation aggregate.
#[tokio::test]
async fn record_and_query_totals() {
    let tmp = NamedTempFile::new().unwrap();
    let store = TokenStore::open(tmp.path()).await.unwrap();

    // Conversation A: 100 input, 50 output
    store
        .record(&TokenEvent {
            conversation_id: "conv-a".into(),
            direction: TokenDirection::Input,
            tokens: 100,
            model: None,
        })
        .await
        .unwrap();
    store
        .record(&TokenEvent {
            conversation_id: "conv-a".into(),
            direction: TokenDirection::Output,
            tokens: 50,
            model: Some("claude-sonnet".into()),
        })
        .await
        .unwrap();

    // Conversation B: 200 input, 80 output
    store
        .record(&TokenEvent {
            conversation_id: "conv-b".into(),
            direction: TokenDirection::Input,
            tokens: 200,
            model: None,
        })
        .await
        .unwrap();
    store
        .record(&TokenEvent {
            conversation_id: "conv-b".into(),
            direction: TokenDirection::Output,
            tokens: 80,
            model: Some("claude-opus".into()),
        })
        .await
        .unwrap();

    let totals = store.lifetime_totals().await.unwrap();
    assert_eq!(
        totals,
        TokenTotals {
            input: 300,
            output: 130
        }
    );
}

/// Catches: query failing on an empty table (e.g. NULL from SUM
/// without COALESCE, or `query_row` returning `QueryReturnedNoRows`).
#[tokio::test]
async fn empty_db_returns_zero_totals() {
    let tmp = NamedTempFile::new().unwrap();
    let store = TokenStore::open(tmp.path()).await.unwrap();

    let totals = store.lifetime_totals().await.unwrap();
    assert_eq!(totals, TokenTotals::default());
}

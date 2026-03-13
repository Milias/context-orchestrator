use super::ApiError;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use std::time::Duration;

#[test]
fn from_response_429_extracts_retry_after() {
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", "5".parse().unwrap());
    let body = r#"{"error":{"message":"rate limit exceeded"}}"#;

    let err = ApiError::from_response(StatusCode::from_u16(429).unwrap(), body, &headers);

    assert!(err.is_retryable());
    assert_eq!(err.retry_after(), Some(Duration::from_secs(5)));
    let msg = err.to_string();
    assert!(msg.contains("429"), "expected 429 in: {msg}");
    assert!(
        msg.contains("rate limit exceeded"),
        "expected message in: {msg}"
    );
}

#[test]
fn from_response_401_is_not_retryable() {
    let body = r#"{"error":{"message":"invalid api key"}}"#;
    let err = ApiError::from_response(StatusCode::from_u16(401).unwrap(), body, &HeaderMap::new());

    assert!(!err.is_retryable());
    assert!(err.to_string().contains("Auth failed"));
    assert!(err.to_string().contains("ANTHROPIC_API_KEY"));
}

#[test]
fn from_response_500_is_retryable_without_retry_after() {
    let body = r#"{"error":{"message":"internal error"}}"#;
    let err = ApiError::from_response(StatusCode::from_u16(500).unwrap(), body, &HeaderMap::new());

    assert!(err.is_retryable());
    assert_eq!(err.retry_after(), None);
}

#[test]
fn from_response_400_is_bad_request() {
    let body = r#"{"error":{"message":"invalid model"}}"#;
    let err = ApiError::from_response(StatusCode::from_u16(400).unwrap(), body, &HeaderMap::new());

    assert!(!err.is_retryable());
    assert!(err.to_string().contains("Bad request"));
}

#[test]
fn from_response_falls_back_to_raw_body() {
    let body = "not json at all";
    let err = ApiError::from_response(StatusCode::from_u16(502).unwrap(), body, &HeaderMap::new());

    assert!(err.is_retryable());
    assert!(
        err.to_string().contains("not json at all"),
        "expected raw body fallback in: {}",
        err
    );
}

#[test]
fn from_response_truncates_long_body() {
    let body = "x".repeat(300);
    let err = ApiError::from_response(StatusCode::from_u16(400).unwrap(), &body, &HeaderMap::new());

    let msg = err.to_string();
    assert!(
        msg.len() < 300,
        "expected truncation, got len={}",
        msg.len()
    );
    assert!(msg.contains("..."));
}

#[test]
fn from_response_529_overloaded() {
    let body = r#"{"error":{"message":"overloaded"}}"#;
    let err = ApiError::from_response(StatusCode::from_u16(529).unwrap(), body, &HeaderMap::new());

    assert!(err.is_retryable());
    assert!(err.to_string().contains("overloaded"));
}

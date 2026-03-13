use super::{with_retry, RetryConfig};
use crate::llm::error::ApiError;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fast_config(max_attempts: u32) -> RetryConfig {
    RetryConfig {
        max_attempts,
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(100),
    }
}

#[tokio::test]
async fn succeeds_on_first_attempt() {
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);

    let result: anyhow::Result<i32> = with_retry(&fast_config(3), || {
        let c = Arc::clone(&calls_clone);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(42)
        }
    })
    .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_on_retryable_error_then_succeeds() {
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);

    let result: anyhow::Result<&str> = with_retry(&fast_config(3), || {
        let c = Arc::clone(&calls_clone);
        async move {
            let n = c.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(ApiError::Timeout.into())
            } else {
                Ok("ok")
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn stops_immediately_on_non_retryable_error() {
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);

    let result: anyhow::Result<i32> = with_retry(&fast_config(3), || {
        let c = Arc::clone(&calls_clone);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err(ApiError::Auth {
                status: 401,
                message: "bad key".into(),
            }
            .into())
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "should not retry auth errors"
    );
}

#[tokio::test]
async fn exhausts_max_attempts() {
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);

    let result: anyhow::Result<i32> = with_retry(&fast_config(3), || {
        let c = Arc::clone(&calls_clone);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err(ApiError::Network("down".into()).into())
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn backoff_delay_increases_exponentially() {
    let config = RetryConfig {
        max_attempts: 3,
        initial_delay: Duration::from_millis(50),
        max_delay: Duration::from_secs(10),
    };

    let d0 = config.delay_for(0, None);
    let d1 = config.delay_for(1, None);
    let d2 = config.delay_for(2, None);

    assert_eq!(d0, Duration::from_millis(50));
    assert_eq!(d1, Duration::from_millis(100));
    assert_eq!(d2, Duration::from_millis(200));
}

#[tokio::test]
async fn backoff_respects_retry_after() {
    let config = fast_config(3);
    let err = ApiError::Retryable {
        status: 429,
        message: "slow down".into(),
        retry_after: Some(Duration::from_secs(5)),
    };

    let delay = config.delay_for(0, Some(&err));
    assert_eq!(
        delay,
        Duration::from_secs(5),
        "should use retry_after when larger"
    );
}

#[tokio::test]
async fn actual_retry_takes_nonzero_time() {
    let start = Instant::now();
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);

    let _ = with_retry(&fast_config(2), || {
        let c = Arc::clone(&calls_clone);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<i32, _>(ApiError::Timeout.into())
        }
    })
    .await;

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(9),
        "expected backoff sleep, elapsed={elapsed:?}"
    );
}

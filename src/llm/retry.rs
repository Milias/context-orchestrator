use super::error::ApiError;
use std::future::Future;
use std::time::Duration;

pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryConfig {
    /// Compute the backoff delay for a given zero-based attempt index,
    /// honoring `retry_after` from the error if present.
    pub fn delay_for(&self, attempt: u32, error: Option<&ApiError>) -> Duration {
        let exponential = self
            .initial_delay
            .saturating_mul(2u32.saturating_pow(attempt));
        let base = exponential.min(self.max_delay);
        match error.and_then(ApiError::retry_after) {
            Some(ra) => base.max(ra),
            None => base,
        }
    }
}

/// Retry an async operation with exponential backoff.
///
/// Retries only when the error contains an [`ApiError`] that is retryable.
/// Non-retryable errors and exhausted attempts return the last error immediately.
pub async fn with_retry<F, Fut, T>(config: &RetryConfig, mut operation: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut last_error = None;

    for attempt in 0..config.max_attempts {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let retryable = e
                    .downcast_ref::<ApiError>()
                    .is_some_and(ApiError::is_retryable);

                if !retryable || attempt + 1 == config.max_attempts {
                    return Err(e);
                }

                let delay = config.delay_for(attempt, e.downcast_ref::<ApiError>());
                last_error = Some(e);
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("retry exhausted with no error")))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

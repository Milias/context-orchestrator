use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum ApiError {
    /// 429, 500, 502, 503, 529 — transient server/rate-limit errors.
    Retryable {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },
    /// 401, 403 — authentication or authorization failure.
    Auth { status: u16, message: String },
    /// 400, 404, 422 — malformed request or invalid endpoint.
    BadRequest { status: u16, message: String },
    /// DNS, connect, TLS, or other transport-level failures.
    Network(String),
    /// Read or connect timeout expired.
    Timeout,
}

impl ApiError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Retryable { .. } | Self::Network(_) | Self::Timeout
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Retryable { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    pub fn from_response(status: StatusCode, body: &str, headers: &HeaderMap) -> Self {
        let message = parse_error_message(body);
        let code = status.as_u16();

        match code {
            401 | 403 => Self::Auth {
                status: code,
                message,
            },
            429 | 500 | 502 | 503 | 529 => Self::Retryable {
                status: code,
                message,
                retry_after: parse_retry_after(headers),
            },
            _ => Self::BadRequest {
                status: code,
                message,
            },
        }
    }

    pub fn from_reqwest(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            return Self::Timeout;
        }
        if e.is_connect() {
            return Self::Network(format!("Connection failed: {e}"));
        }
        Self::Network(format!("{e}"))
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable {
                status, message, ..
            } => {
                let label = match *status {
                    429 => "Rate limited",
                    529 => "API overloaded",
                    _ => "Server error",
                };
                write!(f, "{label} ({status}): {message}")
            }
            Self::Auth {
                status, message, ..
            } => write!(
                f,
                "Auth failed ({status}): {message} — check ANTHROPIC_API_KEY"
            ),
            Self::BadRequest {
                status, message, ..
            } => write!(f, "Bad request ({status}): {message}"),
            Self::Network(msg) => write!(f, "Network error: {msg}"),
            Self::Timeout => write!(f, "Request timed out — API not responding"),
        }
    }
}

impl std::error::Error for ApiError {}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get("retry-after")?.to_str().ok()?;
    let seconds: u64 = value.parse().ok()?;
    Some(Duration::from_secs(seconds))
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error: Option<ErrorDetail>,
}

#[derive(serde::Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

fn parse_error_message(body: &str) -> String {
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|b| b.error)
        .and_then(|e| e.message)
        .unwrap_or_else(|| {
            if body.len() > 200 {
                format!("{}...", &body[..200])
            } else {
                body.to_string()
            }
        })
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

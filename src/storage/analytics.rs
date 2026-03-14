//! Async analytics store backed by `SQLite` via `tokio-rusqlite`.

use crate::storage::schema::{self, TokenDirection, TokenEvent, TokenTotals};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Async analytics store backed by `SQLite`.
///
/// Wraps a [`tokio_rusqlite::Connection`] which runs all SQL on a
/// dedicated background thread. `Clone` is cheap — the connection is
/// a channel handle, not the database itself.
#[derive(Clone)]
pub struct TokenStore {
    conn: tokio_rusqlite::Connection,
}

impl TokenStore {
    /// Open (or create) the analytics database at `path`.
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open(path).await?;
        conn.call(|conn| {
            // Avoid SQLITE_BUSY errors under filesystem contention.
            conn.execute_batch("PRAGMA busy_timeout = 5000")?;
            conn.execute_batch(schema::CREATE_TOKEN_EVENTS)?;
            Ok(())
        })
        .await?;
        Ok(Self { conn })
    }

    /// Open the default analytics database at `~/.context-manager/analytics.db`.
    pub async fn open_default() -> Result<Self> {
        Self::open(&default_analytics_path()?).await
    }

    /// Record a single token-usage event.
    pub async fn record(&self, event: &TokenEvent) -> Result<()> {
        let event = event.clone();
        let now = chrono::Utc::now().to_rfc3339();

        self.conn
            .call(move |conn| {
                conn.execute(
                    schema::INSERT_TOKEN_EVENT,
                    rusqlite::params![
                        event.conversation_id,
                        event.direction.as_str(),
                        event.tokens,
                        event.model,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Query aggregated lifetime totals across all conversations.
    pub async fn lifetime_totals(&self) -> Result<TokenTotals> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(schema::SUM_BY_DIRECTION)?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;

                let mut totals = TokenTotals::default();
                for row in rows {
                    let (direction, total) = row?;
                    let value = u64::try_from(total).unwrap_or(0);
                    match TokenDirection::from_str(&direction) {
                        Some(TokenDirection::Input) => totals.input = value,
                        Some(TokenDirection::Output) => totals.output = value,
                        None => {}
                    }
                }
                Ok(totals)
            })
            .await
            .map_err(Into::into)
    }
}

/// Resolve the default analytics database path, creating the parent
/// directory if it does not exist.
fn default_analytics_path() -> Result<PathBuf> {
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    let dir = PathBuf::from(home).join(".context-manager");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("analytics.db"))
}

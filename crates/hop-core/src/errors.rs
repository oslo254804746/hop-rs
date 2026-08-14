use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, HopCoreError>;

#[derive(Debug, Error)]
pub enum HopCoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error(
        "legacy_database_unsupported: database at {path} is not a Hop v0.2 database; back it up and delete it, or choose a new database path"
    )]
    LegacyDatabaseUnsupported { path: PathBuf },

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

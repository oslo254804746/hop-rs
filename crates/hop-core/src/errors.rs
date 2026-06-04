use thiserror::Error;

pub type Result<T> = std::result::Result<T, HopCoreError>;

#[derive(Debug, Error)]
pub enum HopCoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

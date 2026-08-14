use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

use crate::{HopCoreError, Result};

mod apply;
pub mod manifest;
mod runtime;

pub use apply::{ApplyAction, ApplyChange, ApplyOptions, ApplySummary};
pub use manifest::{CatalogError, CatalogErrorCode, Manifest, MANIFEST_API_VERSION};

#[derive(Debug, Clone, Serialize)]
pub struct CatalogStatus {
    pub revision: i64,
    pub manifest_api_version: &'static str,
    pub sources: Vec<ConfigSourceStatus>,
    pub orphans: Vec<OrphanStatus>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConfigSourceStatus {
    pub source_id: String,
    pub generation: i64,
    pub last_success_at: Option<String>,
    pub last_success_revision: Option<i64>,
    pub last_error_at: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrphanStatus {
    pub resource_type: String,
    pub source_id: String,
    pub source_key: String,
    pub orphaned_at: String,
}

const SCHEMA_VERSION: &str = "hop/v0.2";
const BASELINE_SQL: &str = include_str!("catalog/schema_v0_2.sql");

/// The only storage boundary for v0.2 resources and lightweight runtime state.
///
/// Opening an existing database always performs a read-only schema preflight
/// before a writable connection is created. Databases without the v0.2 marker
/// are rejected without running migrations or changing journal settings.
#[derive(Debug, Clone)]
pub struct Catalog {
    pool: SqlitePool,
}

impl Catalog {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let state = preflight(path).await?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        if state == DatabaseState::Empty {
            initialize(&pool).await?;
        }
        verify_schema(&pool, path).await?;
        Ok(Self { pool })
    }

    pub async fn in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        initialize(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn connect_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if preflight(path).await? != DatabaseState::V0_2 {
            return Err(HopCoreError::Config(format!(
                "v0.2 catalog does not exist at {}",
                path.display()
            )));
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        verify_schema(&pool, path).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn revision(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT revision FROM catalog_meta WHERE singleton_id = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn schema_version(&self) -> Result<String> {
        sqlx::query_scalar("SELECT schema_version FROM hop_schema WHERE singleton_id = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn status(&self) -> std::result::Result<CatalogStatus, CatalogError> {
        let revision = self.revision().await.map_err(status_database_error)?;
        let sources = sqlx::query_as::<_, ConfigSourceStatus>(
            r#"
            SELECT source_id, generation, last_success_at, last_success_revision,
                   last_error_at, last_error_code, last_error_message
            FROM config_sources
            ORDER BY source_id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(status_database_error)?;
        let orphans = sqlx::query_as::<_, OrphanStatus>(
            r#"
            SELECT resource_type, source_id, source_key, orphaned_at
            FROM resource_ownership
            WHERE orphaned_at IS NOT NULL AND source_id IS NOT NULL
            ORDER BY resource_type, source_key
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(status_database_error)?;
        Ok(CatalogStatus {
            revision,
            manifest_api_version: MANIFEST_API_VERSION,
            sources,
            orphans,
        })
    }

    pub async fn record_apply_failure(
        &self,
        source_id: &str,
        actor_label: &str,
        error: &CatalogError,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO config_sources
                (source_id, generation, last_error_at, last_error_code, last_error_message)
            VALUES (?1, 0, CURRENT_TIMESTAMP, ?2, ?3)
            ON CONFLICT(source_id) DO UPDATE SET
                last_error_at = excluded.last_error_at,
                last_error_code = excluded.last_error_code,
                last_error_message = excluded.last_error_message
            "#,
        )
        .bind(source_id)
        .bind(error.code.to_string())
        .bind(&error.message)
        .execute(&mut *transaction)
        .await?;
        let details = serde_json::json!({
            "source_id": source_id,
            "error_code": error.code.to_string(),
            "path": error.path,
        });
        sqlx::query(
            r#"
            INSERT INTO audit_events
                (id, actor_label, action, target_type, target_id, target_label, result, details_json)
            VALUES (?1, ?2, 'config.apply', 'config_source', ?3, ?3, 'failed', ?4)
            "#,
        )
        .bind(crate::new_id())
        .bind(actor_label)
        .bind(source_id)
        .bind(details.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn status_database_error(_: impl std::fmt::Display) -> CatalogError {
    CatalogError::new(
        CatalogErrorCode::ApplyFailed,
        None,
        "catalog database operation failed",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseState {
    Empty,
    V0_2,
}

async fn preflight(path: &Path) -> Result<DatabaseState> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DatabaseState::Empty)
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.len() == 0 {
        return Ok(DatabaseState::Empty);
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let has_tables: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%')",
    )
    .fetch_one(&pool)
    .await?;
    if !has_tables {
        pool.close().await;
        return Ok(DatabaseState::Empty);
    }
    let has_marker: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'hop_schema')",
    )
    .fetch_one(&pool)
    .await?;
    if !has_marker {
        pool.close().await;
        return Err(legacy_database_error(path));
    }
    let version = sqlx::query_scalar::<_, String>(
        "SELECT schema_version FROM hop_schema WHERE singleton_id = 1",
    )
    .fetch_optional(&pool)
    .await?;
    pool.close().await;
    match version.as_deref() {
        Some(SCHEMA_VERSION) => Ok(DatabaseState::V0_2),
        _ => Err(legacy_database_error(path)),
    }
}

async fn initialize(pool: &SqlitePool) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(BASELINE_SQL)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn verify_schema(pool: &SqlitePool, path: &Path) -> Result<()> {
    let version = sqlx::query_scalar::<_, String>(
        "SELECT schema_version FROM hop_schema WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?;
    if version.as_deref() == Some(SCHEMA_VERSION) {
        Ok(())
    } else {
        Err(legacy_database_error(path))
    }
}

fn legacy_database_error(path: &Path) -> HopCoreError {
    HopCoreError::LegacyDatabaseUnsupported {
        path: PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use sqlx::{Connection, Executor, SqliteConnection};
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn initializes_a_fresh_v0_2_database() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("hop.db");

        let catalog = Catalog::connect(&path).await.unwrap();

        assert_eq!(catalog.schema_version().await.unwrap(), SCHEMA_VERSION);
        assert_eq!(catalog.revision().await.unwrap(), 0);
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
                .fetch_all(catalog.pool())
                .await
                .unwrap();
        assert!(tables.contains(&"resource_ownership".to_string()));
        assert!(tables.contains(&"config_sources".to_string()));
        assert!(tables.contains(&"access_key_assets".to_string()));
        assert!(!tables.contains(&"admin_users".to_string()));
        assert!(!tables.contains(&"_sqlx_migrations".to_string()));
    }

    #[tokio::test]
    async fn reopens_an_existing_v0_2_database() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("hop.db");
        let catalog = Catalog::connect(&path).await.unwrap();
        catalog.pool().close().await;

        let reopened = Catalog::connect(&path).await.unwrap();

        assert_eq!(reopened.schema_version().await.unwrap(), SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn read_only_open_does_not_modify_the_catalog_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("hop.db");
        let catalog = Catalog::connect(&path).await.unwrap();
        catalog.pool().close().await;
        let before_bytes = std::fs::read(&path).unwrap();
        let before_modified = modified(&path);

        let read_only = Catalog::connect_read_only(&path).await.unwrap();
        assert_eq!(read_only.revision().await.unwrap(), 0);
        read_only.pool().close().await;

        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        assert_eq!(modified(&path), before_modified);
    }

    #[tokio::test]
    async fn rejects_a_v0_1_database_without_modifying_bytes_or_mtime() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("hop.db");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let mut connection = SqliteConnection::connect(&url).await.unwrap();
        connection
            .execute(
                "CREATE TABLE authorized_keys (id TEXT PRIMARY KEY, public_key TEXT NOT NULL);\
                 INSERT INTO authorized_keys (id, public_key) VALUES ('legacy-key', 'ssh-ed25519 AAAA');",
            )
            .await
            .unwrap();
        connection.close().await.unwrap();
        let before_bytes = std::fs::read(&path).unwrap();
        let before_modified = modified(&path);

        let error = Catalog::connect(&path).await.unwrap_err();

        assert!(matches!(
            error,
            HopCoreError::LegacyDatabaseUnsupported {
                path: ref error_path
            }
                if error_path == &path
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        assert_eq!(modified(&path), before_modified);
        assert!(!path.with_extension("db-wal").exists());
        assert!(!path.with_extension("db-shm").exists());
    }

    fn modified(path: &Path) -> SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }
}

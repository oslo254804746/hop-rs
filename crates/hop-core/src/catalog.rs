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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOwnership {
    Local,
    Config,
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
    v0_3_ready: bool,
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
        } else {
            ensure_v0_3_schema(&pool).await?;
        }
        verify_schema(&pool, path).await?;
        Ok(Self {
            pool,
            v0_3_ready: true,
        })
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
        Ok(Self {
            pool,
            v0_3_ready: true,
        })
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
        let v0_3_ready = check_v0_3_schema(&pool).await?;
        Ok(Self { pool, v0_3_ready })
    }

    pub(crate) fn has_v0_3_schema(&self) -> bool {
        self.v0_3_ready
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
        return Err(unsupported_database_error(path));
    }
    let version = sqlx::query_scalar::<_, String>(
        "SELECT schema_version FROM hop_schema WHERE singleton_id = 1",
    )
    .fetch_optional(&pool)
    .await?;
    pool.close().await;
    match version.as_deref() {
        Some(SCHEMA_VERSION) => Ok(DatabaseState::V0_2),
        _ => Err(unsupported_database_error(path)),
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

async fn has_table<'e, E>(executor: E, table: &str) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
    )
    .bind(table)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

async fn has_column<'e, E>(executor: E, table: &str, column: &str) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let sql = format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1)");
    sqlx::query_scalar(&sql)
        .bind(column)
        .fetch_one(executor)
        .await
        .map_err(Into::into)
}

async fn check_v0_3_schema(pool: &SqlitePool) -> Result<bool> {
    if !has_column(pool, "assets", "via_asset_id").await?
        || !has_column(pool, "assets", "allow_tcp_forwarding").await?
        || !has_table(pool, "asset_health").await?
        || !has_column(pool, "asset_health", "consecutive_failures").await?
        || !has_column(pool, "asset_health", "alert_firing").await?
        || !has_column(pool, "asset_health", "last_alerted_at").await?
        || !has_column(pool, "asset_health", "last_recovered_at").await?
        || !has_table(pool, "asset_probe_samples").await?
        || !has_table(pool, "probe_settings").await?
        || !has_table(pool, "notification_channels").await?
        || !has_table(pool, "audit_events").await?
    {
        return Ok(false);
    }
    Ok(true)
}

async fn ensure_v0_3_schema(pool: &SqlitePool) -> Result<()> {
    if check_v0_3_schema(pool).await? {
        return Ok(());
    }

    let mut transaction = pool.begin().await?;

    if !has_column(&mut *transaction, "assets", "via_asset_id").await? {
        sqlx::query("ALTER TABLE assets ADD COLUMN via_asset_id TEXT REFERENCES assets(id) ON DELETE RESTRICT")
            .execute(&mut *transaction)
            .await?;
    }
    if !has_column(&mut *transaction, "assets", "allow_tcp_forwarding").await? {
        sqlx::query("ALTER TABLE assets ADD COLUMN allow_tcp_forwarding INTEGER NOT NULL DEFAULT 1 CHECK (allow_tcp_forwarding IN (0, 1))")
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_assets_via_asset_id ON assets(via_asset_id)")
        .execute(&mut *transaction)
        .await?;

    sqlx::raw_sql(
        r#"
        CREATE TABLE IF NOT EXISTS asset_health (
            asset_id             TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
            status               TEXT NOT NULL DEFAULT 'unknown' CHECK (status IN ('unknown', 'healthy', 'failed')),
            checked_at           TEXT,
            last_success_at      TEXT,
            latency_ms           INTEGER,
            error_code           TEXT,
            error_message        TEXT,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            alert_firing         INTEGER NOT NULL DEFAULT 0 CHECK (alert_firing IN (0, 1)),
            last_alerted_at      TEXT,
            last_recovered_at    TEXT
        );

        CREATE TRIGGER IF NOT EXISTS assets_create_health
        AFTER INSERT ON assets
        BEGIN
            INSERT OR IGNORE INTO asset_health (asset_id) VALUES (NEW.id);
        END;

        INSERT OR IGNORE INTO asset_health (asset_id)
        SELECT id FROM assets;
        "#,
    )
    .execute(&mut *transaction)
    .await?;

    if !has_column(&mut *transaction, "asset_health", "consecutive_failures").await? {
        sqlx::query(
            "ALTER TABLE asset_health ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&mut *transaction)
        .await?;
    }
    if !has_column(&mut *transaction, "asset_health", "alert_firing").await? {
        sqlx::query(
            "ALTER TABLE asset_health ADD COLUMN alert_firing INTEGER NOT NULL DEFAULT 0 CHECK (alert_firing IN (0, 1))",
        )
        .execute(&mut *transaction)
        .await?;
    }
    if !has_column(&mut *transaction, "asset_health", "last_alerted_at").await? {
        sqlx::query("ALTER TABLE asset_health ADD COLUMN last_alerted_at TEXT")
            .execute(&mut *transaction)
            .await?;
    }
    if !has_column(&mut *transaction, "asset_health", "last_recovered_at").await? {
        sqlx::query("ALTER TABLE asset_health ADD COLUMN last_recovered_at TEXT")
            .execute(&mut *transaction)
            .await?;
    }

    sqlx::raw_sql(
        r#"
        CREATE TABLE IF NOT EXISTS asset_probe_samples (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            asset_id   TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
            checked_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            status     TEXT NOT NULL CHECK (status IN ('healthy', 'failed')),
            latency_ms INTEGER,
            error_code TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_asset_probe_samples_asset_id
        ON asset_probe_samples(asset_id, id DESC);

        CREATE TABLE IF NOT EXISTS probe_settings (
            singleton_id       INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            enabled            INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            interval_seconds   INTEGER NOT NULL DEFAULT 300 CHECK (interval_seconds BETWEEN 30 AND 86400),
            timeout_ms         INTEGER NOT NULL DEFAULT 4000 CHECK (timeout_ms BETWEEN 500 AND 15000),
            failure_threshold  INTEGER NOT NULL DEFAULT 2 CHECK (failure_threshold BETWEEN 1 AND 10),
            notify_on_recovery INTEGER NOT NULL DEFAULT 1 CHECK (notify_on_recovery IN (0, 1)),
            updated_at         TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        INSERT OR IGNORE INTO probe_settings (singleton_id) VALUES (1);

        CREATE TABLE IF NOT EXISTS notification_channels (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            kind       TEXT NOT NULL CHECK (kind IN ('telegram', 'bark', 'webhook')),
            enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            config_enc TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS audit_events (
            id           TEXT PRIMARY KEY,
            occurred_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            actor_label  TEXT NOT NULL,
            action       TEXT NOT NULL,
            target_type  TEXT NOT NULL,
            target_id    TEXT,
            target_label TEXT,
            result       TEXT NOT NULL,
            source_ip    TEXT,
            details_json TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_audit_events_occurred_at
        ON audit_events(occurred_at DESC);
        "#,
    )
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
        Err(unsupported_database_error(path))
    }
}

fn unsupported_database_error(path: &Path) -> HopCoreError {
    HopCoreError::UnsupportedDatabaseSchema {
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
        assert!(tables.contains(&"asset_probe_samples".to_string()));
        assert!(tables.contains(&"probe_settings".to_string()));
        assert!(tables.contains(&"notification_channels".to_string()));
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
        for _ in 0..8 {
            let directory = tempdir().unwrap();
            let path = directory.path().join("hop.db");
            let catalog = Catalog::connect(&path).await.unwrap();
            let checkpoint =
                sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
                    .fetch_one(catalog.pool())
                    .await
                    .unwrap();
            assert_eq!(checkpoint.0, 0, "WAL checkpoint must not be busy");
            catalog.pool().close().await;
            let before_bytes = std::fs::read(&path).unwrap();
            let before_modified = modified(&path);

            let read_only = Catalog::connect_read_only(&path).await.unwrap();
            assert_eq!(read_only.revision().await.unwrap(), 0);
            read_only.pool().close().await;

            assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
            assert_eq!(modified(&path), before_modified);
        }
    }

    #[tokio::test]
    async fn upgrades_v0_2_9_database_in_place_and_allows_read_only_before_upgrade() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("hop.db");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let mut connection = SqliteConnection::connect(&url).await.unwrap();
        connection
            .execute(
                r#"
                CREATE TABLE hop_schema (
                    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
                    schema_version TEXT NOT NULL CHECK (schema_version = 'hop/v0.2')
                );
                INSERT INTO hop_schema (singleton_id, schema_version) VALUES (1, 'hop/v0.2');
                CREATE TABLE catalog_meta (
                    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
                    revision INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO catalog_meta (singleton_id, revision) VALUES (1, 3);
                CREATE TABLE credentials (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    username TEXT NOT NULL,
                    auth_type TEXT NOT NULL,
                    password_enc TEXT,
                    private_key_enc TEXT,
                    passphrase_enc TEXT,
                    secret_hmac TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE assets (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    asset_type TEXT NOT NULL,
                    host TEXT NOT NULL,
                    port INTEGER NOT NULL,
                    display_name TEXT,
                    description TEXT,
                    tags_json TEXT,
                    credential_id TEXT REFERENCES credentials(id) ON DELETE RESTRICT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE asset_health (
                    asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
                    status TEXT NOT NULL DEFAULT 'unknown',
                    checked_at TEXT,
                    last_success_at TEXT,
                    latency_ms INTEGER,
                    error_code TEXT,
                    error_message TEXT
                );
                INSERT INTO assets (id, name, asset_type, host, port)
                VALUES ('asset-1', 'hk-entry', 'ssh', '198.51.100.10', 22);
                INSERT INTO asset_health (asset_id, status, latency_ms)
                VALUES ('asset-1', 'healthy', 32);
                "#,
            )
            .await
            .unwrap();
        connection.close().await.unwrap();

        let before_bytes = std::fs::read(&path).unwrap();
        let before_modified = modified(&path);
        let ro = Catalog::connect_read_only(&path).await.unwrap();
        let ro_assets = ro.list_assets().await.unwrap();
        assert_eq!(ro_assets.len(), 1);
        assert_eq!(ro_assets[0].name, "hk-entry");
        assert!(ro_assets[0].allow_tcp_forwarding);
        assert_eq!(ro_assets[0].via_asset_id, None);
        ro.pool().close().await;
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        assert_eq!(modified(&path), before_modified);

        let migrated = Catalog::connect(&path).await.unwrap();
        assert_eq!(migrated.revision().await.unwrap(), 3);
        let assets = migrated.list_assets().await.unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].health.latency_ms, Some(32));
        assert!(assets[0].allow_tcp_forwarding);
    }

    #[tokio::test]
    async fn rejects_an_unrecognized_database_without_modifying_bytes_or_mtime() {
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
            HopCoreError::UnsupportedDatabaseSchema {
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

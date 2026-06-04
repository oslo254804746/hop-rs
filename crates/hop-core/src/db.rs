use std::{path::Path, time::Duration};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

use crate::{
    errors::HopCoreError,
    models::{
        new_id, validate_tcp_port, Asset, AssetRow, AuthorizedKey, Credential, KnownHost, NewAsset,
        NewAuthorizedKey, NewCredential, NewKnownHost, NewSession, Session, Setting,
    },
    Result,
};

#[derive(Debug, Clone)]
pub struct HopDb {
    pool: SqlitePool,
}

impl HopDb {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_with_options(options, 5).await
    }

    pub async fn in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_with_options(options, 1).await
    }

    async fn connect_with_options(
        options: SqliteConnectOptions,
        max_connections: u32,
    ) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await?;
        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn add_authorized_key(&self, new_key: NewAuthorizedKey) -> Result<AuthorizedKey> {
        let id = new_id();
        sqlx::query(
            r#"
            INSERT INTO authorized_keys (id, name, public_key, fingerprint, is_active)
            VALUES (?1, ?2, ?3, ?4, TRUE)
            "#,
        )
        .bind(&id)
        .bind(new_key.name)
        .bind(new_key.public_key)
        .bind(new_key.fingerprint)
        .execute(&self.pool)
        .await?;
        self.get_authorized_key_by_id(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn list_authorized_keys(&self) -> Result<Vec<AuthorizedKey>> {
        sqlx::query_as::<_, AuthorizedKey>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, created_at
            FROM authorized_keys
            ORDER BY created_at DESC, name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_active_authorized_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<AuthorizedKey>> {
        sqlx::query_as::<_, AuthorizedKey>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, created_at
            FROM authorized_keys
            WHERE fingerprint = ?1 AND is_active = TRUE
            "#,
        )
        .bind(fingerprint)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_authorized_key_by_id(&self, id: &str) -> Result<Option<AuthorizedKey>> {
        sqlx::query_as::<_, AuthorizedKey>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, created_at
            FROM authorized_keys
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn set_authorized_key_active(&self, id: &str, is_active: bool) -> Result<()> {
        sqlx::query("UPDATE authorized_keys SET is_active = ?1 WHERE id = ?2")
            .bind(is_active)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_authorized_key(&self, id: &str, key: NewAuthorizedKey) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE authorized_keys
            SET name = ?1, public_key = ?2, fingerprint = ?3
            WHERE id = ?4
            "#,
        )
        .bind(key.name)
        .bind(key.public_key)
        .bind(key.fingerprint)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_authorized_key(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM authorized_keys WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_credential(&self, credential: NewCredential) -> Result<Credential> {
        let id = credential.id.unwrap_or_else(new_id);
        sqlx::query(
            r#"
            INSERT INTO credentials (id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&id)
        .bind(credential.name)
        .bind(credential.username)
        .bind(credential.auth_type.as_str())
        .bind(credential.password_enc)
        .bind(credential.private_key_enc)
        .bind(credential.passphrase_enc)
        .execute(&self.pool)
        .await?;
        self.get_credential(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        sqlx::query_as::<_, Credential>(
            r#"
            SELECT id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc, created_at
            FROM credentials
            ORDER BY created_at DESC, name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        sqlx::query_as::<_, Credential>(
            r#"
            SELECT id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc, created_at
            FROM credentials
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_credential(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM credentials WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_credential(&self, id: &str, credential: NewCredential) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE credentials
            SET name = ?1,
                username = ?2,
                auth_type = ?3,
                password_enc = ?4,
                private_key_enc = ?5,
                passphrase_enc = ?6
            WHERE id = ?7
            "#,
        )
        .bind(credential.name)
        .bind(credential.username)
        .bind(credential.auth_type.as_str())
        .bind(credential.password_enc)
        .bind(credential.private_key_enc)
        .bind(credential.passphrase_enc)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_asset(&self, asset: NewAsset) -> Result<Asset> {
        let id = new_id();
        let port = validate_tcp_port(asset.port)?;
        let tags = serde_json::to_string(&asset.tags)?;
        sqlx::query(
            r#"
            INSERT INTO assets (id, name, hostname, port, description, tags, credential_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&id)
        .bind(asset.name)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(asset.description)
        .bind(tags)
        .bind(asset.credential_id)
        .execute(&self.pool)
        .await?;
        self.get_asset_by_id(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn update_asset(&self, id: &str, asset: NewAsset) -> Result<()> {
        let port = validate_tcp_port(asset.port)?;
        let tags = serde_json::to_string(&asset.tags)?;
        sqlx::query(
            r#"
            UPDATE assets
            SET name = ?1,
                hostname = ?2,
                port = ?3,
                description = ?4,
                tags = ?5,
                credential_id = ?6,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?7
            "#,
        )
        .bind(asset.name)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(asset.description)
        .bind(tags)
        .bind(asset.credential_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_asset(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM assets WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_assets(&self) -> Result<Vec<Asset>> {
        let rows = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            ORDER BY name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(Asset::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub async fn get_asset_by_id(&self, id: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose().map_err(Into::into)
    }

    pub async fn get_asset_by_name(&self, name: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            WHERE name = ?1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose().map_err(Into::into)
    }

    pub async fn find_proxy_asset(
        &self,
        host_to_connect: &str,
        port: i64,
    ) -> Result<Option<Asset>> {
        let normalized_name = host_to_connect
            .strip_suffix(".hop")
            .unwrap_or(host_to_connect);

        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            WHERE (hostname = ?1 AND port = ?2)
               OR (name = ?3)
            ORDER BY CASE WHEN hostname = ?1 AND port = ?2 THEN 0 ELSE 1 END
            LIMIT 1
            "#,
        )
        .bind(host_to_connect)
        .bind(port)
        .bind(normalized_name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose().map_err(Into::into)
    }

    pub async fn start_session(&self, session: NewSession) -> Result<Session> {
        let id = new_id();
        sqlx::query(
            r#"
            INSERT INTO sessions (id, key_finger, key_name, mode, asset_name, target_host, target_port, client_ip, status)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'started')
            "#,
        )
        .bind(&id)
        .bind(session.key_finger)
        .bind(session.key_name)
        .bind(session.mode)
        .bind(session.asset_name)
        .bind(session.target_host)
        .bind(session.target_port)
        .bind(session.client_ip)
        .execute(&self.pool)
        .await?;
        self.get_session(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn finish_session(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET status = ?1, error = ?2, ended_at = CURRENT_TIMESTAMP
            WHERE id = ?3
            "#,
        )
        .bind(status)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        sqlx::query_as::<_, Session>(
            r#"
            SELECT id, key_finger, key_name, mode, asset_name, target_host, target_port,
                   client_ip, status, error, started_at, ended_at
            FROM sessions
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<Session>> {
        sqlx::query_as::<_, Session>(
            r#"
            SELECT id, key_finger, key_name, mode, asset_name, target_host, target_port,
                   client_ip, status, error, started_at, ended_at
            FROM sessions
            ORDER BY started_at DESC
            LIMIT ?1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_known_host(
        &self,
        hostname: &str,
        port: i64,
        key_type: &str,
    ) -> Result<Option<KnownHost>> {
        sqlx::query_as::<_, KnownHost>(
            r#"
            SELECT hostname, port, key_type, fingerprint, first_seen
            FROM known_hosts
            WHERE hostname = ?1 AND port = ?2 AND key_type = ?3
            "#,
        )
        .bind(hostname)
        .bind(port)
        .bind(key_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn upsert_known_host(&self, host: NewKnownHost) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO known_hosts (hostname, port, key_type, fingerprint)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(hostname, port, key_type) DO UPDATE SET fingerprint = excluded.fingerprint
            "#,
        )
        .bind(host.hostname)
        .bind(host.port)
        .bind(host.key_type)
        .bind(host.fingerprint)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_known_hosts(&self) -> Result<Vec<KnownHost>> {
        sqlx::query_as::<_, KnownHost>(
            "SELECT hostname, port, key_type, fingerprint, first_seen FROM known_hosts ORDER BY hostname, port",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_known_host(&self, hostname: &str, port: i64, key_type: &str) -> Result<()> {
        sqlx::query("DELETE FROM known_hosts WHERE hostname = ?1 AND port = ?2 AND key_type = ?3")
            .bind(hostname)
            .bind(port)
            .bind(key_type)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let setting =
            sqlx::query_as::<_, Setting>("SELECT key, value FROM settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(setting.map(|setting| setting.value))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO settings (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{AuthType, NewCredential};

    use super::*;

    #[tokio::test]
    async fn migration_creates_six_mvp_tables() {
        let db = HopDb::in_memory().await.unwrap();
        let tables: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT name FROM sqlite_master
            WHERE type = 'table' AND name NOT LIKE '_sqlx_%'
            ORDER BY name
            "#,
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        let names: Vec<String> = tables.into_iter().map(|row| row.0).collect();
        assert_eq!(
            names,
            vec![
                "assets",
                "authorized_keys",
                "credentials",
                "known_hosts",
                "sessions",
                "settings"
            ]
        );
    }

    #[tokio::test]
    async fn authorized_key_crud_respects_active_flag() {
        let db = HopDb::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA test",
                "SHA256:abc",
            ))
            .await
            .unwrap();

        assert!(db
            .get_active_authorized_key_by_fingerprint("SHA256:abc")
            .await
            .unwrap()
            .is_some());
        db.set_authorized_key_active(&key.id, false).await.unwrap();
        assert!(db
            .get_active_authorized_key_by_fingerprint("SHA256:abc")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn asset_and_credential_crud_roundtrip() {
        let db = HopDb::in_memory().await.unwrap();
        let credential = db
            .add_credential(NewCredential {
                id: None,
                name: "deploy key".to_string(),
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("v1:xchacha20poly1305:a:b".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let mut asset = NewAsset::new("web-prod-01", "10.0.1.10", 22);
        asset.tags = vec!["prod".to_string(), "web".to_string()];
        asset.credential_id = Some(credential.id.clone());
        let inserted = db.add_asset(asset).await.unwrap();

        let listed = db.list_assets().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].tags, vec!["prod", "web"]);
        assert_eq!(
            db.get_asset_by_name("web-prod-01")
                .await
                .unwrap()
                .unwrap()
                .id,
            inserted.id
        );
        assert_eq!(
            db.get_credential(&credential.id)
                .await
                .unwrap()
                .unwrap()
                .username,
            "deploy"
        );
    }

    #[tokio::test]
    async fn proxy_asset_allowlist_matches_only_design_rules() {
        let db = HopDb::in_memory().await.unwrap();
        db.add_asset(NewAsset::new("web-prod-01", "10.0.1.10", 2222))
            .await
            .unwrap();

        assert!(db
            .find_proxy_asset("10.0.1.10", 2222)
            .await
            .unwrap()
            .is_some());
        assert!(db
            .find_proxy_asset("web-prod-01", 22)
            .await
            .unwrap()
            .is_some());
        assert!(db
            .find_proxy_asset("web-prod-01.hop", 22)
            .await
            .unwrap()
            .is_some());
        assert!(db
            .find_proxy_asset("10.0.1.10", 22)
            .await
            .unwrap()
            .is_none());
        assert!(db
            .find_proxy_asset("unlisted.internal", 22)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn sessions_can_be_started_and_finished() {
        let db = HopDb::in_memory().await.unwrap();
        let session = db
            .start_session(NewSession {
                key_finger: "SHA256:key".to_string(),
                key_name: Some("laptop".to_string()),
                mode: "proxyjump".to_string(),
                asset_name: Some("web".to_string()),
                target_host: Some("10.0.1.10".to_string()),
                target_port: Some(22),
                client_ip: Some("127.0.0.1:12345".to_string()),
            })
            .await
            .unwrap();

        db.finish_session(&session.id, "failed", Some("rejected"))
            .await
            .unwrap();
        let finished = db.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(finished.status, "failed");
        assert_eq!(finished.error.as_deref(), Some("rejected"));
        assert!(finished.ended_at.is_some());
    }
}

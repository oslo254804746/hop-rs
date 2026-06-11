use std::{collections::BTreeSet, path::Path, time::Duration};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

use crate::{
    errors::HopCoreError,
    models::{
        new_id, normalize_asset_protocol, protocol_supports_managed_credentials,
        validate_credential_material, validate_tcp_port, Asset, AssetAccessMode, AssetRow,
        AuthorizedKey, AuthorizedKeyRow, Credential, KnownHost, NewAsset, NewAuthorizedKey,
        NewCredential, NewKnownHost, NewSession, Session, Setting,
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
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_with_options(options, 5).await
    }

    pub async fn in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .foreign_keys(true)
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
            INSERT INTO authorized_keys
                (id, name, public_key, fingerprint, is_active, asset_access_mode)
            VALUES (?1, ?2, ?3, ?4, TRUE, ?5)
            "#,
        )
        .bind(&id)
        .bind(new_key.name)
        .bind(new_key.public_key)
        .bind(new_key.fingerprint)
        .bind(new_key.asset_access_mode.as_str())
        .execute(&self.pool)
        .await?;
        self.get_authorized_key_by_id(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn list_authorized_keys(&self) -> Result<Vec<AuthorizedKey>> {
        let rows = sqlx::query_as::<_, AuthorizedKeyRow>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, asset_access_mode, created_at
            FROM authorized_keys
            ORDER BY created_at DESC, name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(AuthorizedKey::try_from).collect()
    }

    pub async fn get_active_authorized_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<AuthorizedKey>> {
        let row = sqlx::query_as::<_, AuthorizedKeyRow>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, asset_access_mode, created_at
            FROM authorized_keys
            WHERE fingerprint = ?1 AND is_active = TRUE
            "#,
        )
        .bind(fingerprint)
        .fetch_optional(&self.pool)
        .await?;
        row.map(AuthorizedKey::try_from).transpose()
    }

    pub async fn get_authorized_key_by_id(&self, id: &str) -> Result<Option<AuthorizedKey>> {
        let row = sqlx::query_as::<_, AuthorizedKeyRow>(
            r#"
            SELECT id, name, public_key, fingerprint, is_active, asset_access_mode, created_at
            FROM authorized_keys
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(AuthorizedKey::try_from).transpose()
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

    pub async fn update_authorized_key_with_access(
        &self,
        id: &str,
        key: NewAuthorizedKey,
        mode: AssetAccessMode,
        asset_ids: &[String],
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        Self::validate_access_assignment(&mut transaction, id, asset_ids).await?;
        let result = sqlx::query(
            r#"
            UPDATE authorized_keys
            SET name = ?1, public_key = ?2, fingerprint = ?3, asset_access_mode = ?4
            WHERE id = ?5
            "#,
        )
        .bind(key.name)
        .bind(key.public_key)
        .bind(key.fingerprint)
        .bind(mode.as_str())
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(HopCoreError::Validation(format!(
                "unknown authorized key id: {id}"
            )));
        }
        Self::replace_access_assignments(&mut transaction, id, asset_ids).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn set_authorized_key_access(
        &self,
        id: &str,
        mode: AssetAccessMode,
        asset_ids: &[String],
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        Self::validate_access_assignment(&mut transaction, id, asset_ids).await?;
        let result = sqlx::query("UPDATE authorized_keys SET asset_access_mode = ?1 WHERE id = ?2")
            .bind(mode.as_str())
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() != 1 {
            return Err(HopCoreError::Validation(format!(
                "unknown authorized key id: {id}"
            )));
        }
        Self::replace_access_assignments(&mut transaction, id, asset_ids).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn validate_access_assignment(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        key_id: &str,
        asset_ids: &[String],
    ) -> Result<()> {
        let key_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM authorized_keys WHERE id = ?1)")
                .bind(key_id)
                .fetch_one(&mut **transaction)
                .await?;
        if !key_exists {
            return Err(HopCoreError::Validation(format!(
                "unknown authorized key id: {key_id}"
            )));
        }
        for asset_id in BTreeSet::from_iter(asset_ids.iter().map(String::as_str)) {
            let asset_exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM assets WHERE id = ?1)")
                    .bind(asset_id)
                    .fetch_one(&mut **transaction)
                    .await?;
            if !asset_exists {
                return Err(HopCoreError::Validation(format!(
                    "unknown asset id: {asset_id}"
                )));
            }
        }
        Ok(())
    }

    async fn replace_access_assignments(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        key_id: &str,
        asset_ids: &[String],
    ) -> Result<()> {
        sqlx::query("DELETE FROM authorized_key_assets WHERE key_id = ?1")
            .bind(key_id)
            .execute(&mut **transaction)
            .await?;
        for asset_id in BTreeSet::from_iter(asset_ids.iter().map(String::as_str)) {
            sqlx::query("INSERT INTO authorized_key_assets (key_id, asset_id) VALUES (?1, ?2)")
                .bind(key_id)
                .bind(asset_id)
                .execute(&mut **transaction)
                .await?;
        }
        Ok(())
    }

    pub async fn list_asset_ids_for_key(&self, key_id: &str) -> Result<Vec<String>> {
        sqlx::query_scalar(
            r#"
            SELECT asset_id
            FROM authorized_key_assets
            WHERE key_id = ?1
            ORDER BY asset_id
            "#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_authorized_key(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM authorized_key_assets WHERE key_id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM authorized_keys WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_credential(&self, credential: NewCredential) -> Result<Credential> {
        validate_credential_material(
            &credential.auth_type,
            credential.password_enc.as_deref(),
            credential.private_key_enc.as_deref(),
            credential.passphrase_enc.as_deref(),
        )?;
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
        validate_credential_material(
            &credential.auth_type,
            credential.password_enc.as_deref(),
            credential.private_key_enc.as_deref(),
            credential.passphrase_enc.as_deref(),
        )?;
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
        let (protocol, preset) =
            normalize_asset_protocol(&asset.protocol, asset.preset.as_deref())?;
        let port = validate_tcp_port(asset.port)?;
        let tags = serde_json::to_string(&asset.tags)?;
        let credential_id = if protocol_supports_managed_credentials(&protocol) {
            asset.credential_id
        } else {
            None
        };
        sqlx::query(
            r#"
            INSERT INTO assets (id, name, protocol, preset, hostname, port, description, tags, credential_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(&id)
        .bind(asset.name)
        .bind(protocol)
        .bind(preset)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(asset.description)
        .bind(tags)
        .bind(credential_id)
        .execute(&self.pool)
        .await?;
        self.get_asset_by_id(&id)
            .await?
            .ok_or_else(|| HopCoreError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn update_asset(&self, id: &str, asset: NewAsset) -> Result<()> {
        let (protocol, preset) =
            normalize_asset_protocol(&asset.protocol, asset.preset.as_deref())?;
        let port = validate_tcp_port(asset.port)?;
        let tags = serde_json::to_string(&asset.tags)?;
        let credential_id = if protocol_supports_managed_credentials(&protocol) {
            asset.credential_id
        } else {
            None
        };
        sqlx::query(
            r#"
            UPDATE assets
            SET name = ?1,
                protocol = ?2,
                preset = ?3,
                hostname = ?4,
                port = ?5,
                description = ?6,
                tags = ?7,
                credential_id = ?8,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?9
            "#,
        )
        .bind(asset.name)
        .bind(protocol)
        .bind(preset)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(asset.description)
        .bind(tags)
        .bind(credential_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_asset(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM authorized_key_assets WHERE asset_id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM assets WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_assets(&self) -> Result<Vec<Asset>> {
        let rows = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, protocol, preset, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            ORDER BY name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(Asset::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
    }

    pub async fn get_asset_by_id(&self, id: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, protocol, preset, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn get_asset_by_name(&self, name: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, name, protocol, preset, hostname, port, description, tags, credential_id, created_at, updated_at
            FROM assets
            WHERE name = ?1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn list_assets_for_key(&self, key_id: &str) -> Result<Vec<Asset>> {
        let rows = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.id, a.name, a.protocol, a.preset, a.hostname, a.port, a.description,
                   a.tags, a.credential_id, a.created_at, a.updated_at
            FROM assets a
            JOIN authorized_keys k ON k.id = ?1 AND k.is_active = TRUE
            WHERE k.asset_access_mode = 'all'
               OR EXISTS (
                    SELECT 1 FROM authorized_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id
               )
            ORDER BY a.name ASC
            "#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Asset::try_from).collect()
    }

    pub async fn find_direct_asset_for_key(
        &self,
        key_id: &str,
        target: &str,
    ) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.id, a.name, a.protocol, a.preset, a.hostname, a.port, a.description,
                   a.tags, a.credential_id, a.created_at, a.updated_at
            FROM assets a
            JOIN authorized_keys k ON k.id = ?1 AND k.is_active = TRUE
            WHERE (k.asset_access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM authorized_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id
                  ))
              AND (a.name = ?2 OR a.hostname = ?2)
            ORDER BY CASE WHEN a.name = ?2 THEN 0 ELSE 1 END, a.name ASC
            LIMIT 1
            "#,
        )
        .bind(key_id)
        .bind(target)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn key_can_access_asset(&self, key_id: &str, asset_id: &str) -> Result<bool> {
        sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM authorized_keys k
                JOIN assets a ON a.id = ?2
                WHERE k.id = ?1
                  AND k.is_active = TRUE
                  AND (k.asset_access_mode = 'all' OR EXISTS (
                        SELECT 1 FROM authorized_key_assets aka
                        WHERE aka.key_id = k.id AND aka.asset_id = a.id
                  ))
            )
            "#,
        )
        .bind(key_id)
        .bind(asset_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
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
            SELECT id, name, protocol, preset, hostname, port, description, tags, credential_id, created_at, updated_at
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
        row.map(Asset::try_from).transpose()
    }

    pub async fn find_proxy_asset_for_key(
        &self,
        key_id: &str,
        host_to_connect: &str,
        port: i64,
    ) -> Result<Option<Asset>> {
        let normalized_name = host_to_connect
            .strip_suffix(".hop")
            .unwrap_or(host_to_connect);
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.id, a.name, a.protocol, a.preset, a.hostname, a.port, a.description,
                   a.tags, a.credential_id, a.created_at, a.updated_at
            FROM assets a
            JOIN authorized_keys k ON k.id = ?1 AND k.is_active = TRUE
            WHERE (k.asset_access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM authorized_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id
                  ))
              AND ((a.hostname = ?2 AND a.port = ?3) OR a.name = ?4)
            ORDER BY CASE WHEN a.hostname = ?2 AND a.port = ?3 THEN 0 ELSE 1 END, a.name ASC
            LIMIT 1
            "#,
        )
        .bind(key_id)
        .bind(host_to_connect)
        .bind(port)
        .bind(normalized_name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose()
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
    use crate::models::{AssetAccessMode, AuthType, NewCredential};

    use super::*;

    #[tokio::test]
    async fn migration_creates_authorization_tables() {
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
                "authorized_key_assets",
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

        assert_eq!(key.asset_access_mode, AssetAccessMode::All);

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

    async fn add_test_key(db: &HopDb, name: &str) -> AuthorizedKey {
        db.add_authorized_key(NewAuthorizedKey::new(
            name,
            format!("ssh-ed25519 AAAA-{name}"),
            format!("SHA256:{name}"),
        ))
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn asset_access_replaces_and_deduplicates_assignments() {
        let db = HopDb::in_memory().await.unwrap();
        let key = add_test_key(&db, "restricted").await;
        let first = db
            .add_asset(NewAsset::new("first", "10.0.0.1", 22))
            .await
            .unwrap();
        let second = db
            .add_asset(NewAsset::new("second", "10.0.0.2", 22))
            .await
            .unwrap();

        db.set_authorized_key_access(
            &key.id,
            AssetAccessMode::Restricted,
            &[first.id.clone(), first.id.clone(), second.id.clone()],
        )
        .await
        .unwrap();

        let mut expected = vec![first.id.clone(), second.id.clone()];
        expected.sort();
        assert_eq!(db.list_asset_ids_for_key(&key.id).await.unwrap(), expected);
        assert_eq!(
            db.get_authorized_key_by_id(&key.id)
                .await
                .unwrap()
                .unwrap()
                .asset_access_mode,
            AssetAccessMode::Restricted
        );
        assert!(sqlx::query(
            "INSERT INTO authorized_key_assets (key_id, asset_id) VALUES (?1, ?2)"
        )
        .bind(&key.id)
        .bind(&first.id)
        .execute(db.pool())
        .await
        .is_err());
    }

    #[tokio::test]
    async fn asset_access_rejects_unknown_ids_without_partial_update() {
        let db = HopDb::in_memory().await.unwrap();
        let key = add_test_key(&db, "restricted").await;
        let asset = db
            .add_asset(NewAsset::new("first", "10.0.0.1", 22))
            .await
            .unwrap();
        db.set_authorized_key_access(
            &key.id,
            AssetAccessMode::Restricted,
            std::slice::from_ref(&asset.id),
        )
        .await
        .unwrap();

        assert!(db
            .set_authorized_key_access(
                &key.id,
                AssetAccessMode::All,
                &["missing-asset".to_string()],
            )
            .await
            .is_err());
        let unchanged = db.get_authorized_key_by_id(&key.id).await.unwrap().unwrap();
        assert_eq!(unchanged.asset_access_mode, AssetAccessMode::Restricted);
        assert_eq!(
            db.list_asset_ids_for_key(&key.id).await.unwrap(),
            vec![asset.id]
        );
        assert!(db
            .set_authorized_key_access("missing-key", AssetAccessMode::Restricted, &[],)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn asset_access_assignments_are_removed_with_key_or_asset() {
        let db = HopDb::in_memory().await.unwrap();
        let key = add_test_key(&db, "restricted").await;
        let first = db
            .add_asset(NewAsset::new("first", "10.0.0.1", 22))
            .await
            .unwrap();
        let second = db
            .add_asset(NewAsset::new("second", "10.0.0.2", 22))
            .await
            .unwrap();
        db.set_authorized_key_access(
            &key.id,
            AssetAccessMode::Restricted,
            &[first.id.clone(), second.id.clone()],
        )
        .await
        .unwrap();

        db.delete_asset(&first.id).await.unwrap();
        assert_eq!(
            db.list_asset_ids_for_key(&key.id).await.unwrap(),
            vec![second.id]
        );
        db.delete_authorized_key(&key.id).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM authorized_key_assets")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn asset_access_all_and_restricted_queries_enforce_scope() {
        let db = HopDb::in_memory().await.unwrap();
        let all_key = add_test_key(&db, "all").await;
        let restricted_key = add_test_key(&db, "restricted").await;
        let first = db
            .add_asset(NewAsset::new("first", "shared.internal", 22))
            .await
            .unwrap();
        let second = db
            .add_asset(NewAsset::new("second", "shared.internal", 2222))
            .await
            .unwrap();
        db.set_authorized_key_access(
            &restricted_key.id,
            AssetAccessMode::Restricted,
            std::slice::from_ref(&first.id),
        )
        .await
        .unwrap();

        assert_eq!(db.list_assets_for_key(&all_key.id).await.unwrap().len(), 2);
        let restricted = db.list_assets_for_key(&restricted_key.id).await.unwrap();
        assert_eq!(restricted.len(), 1);
        assert_eq!(restricted[0].id, first.id);
        assert!(db
            .find_direct_asset_for_key(&restricted_key.id, "first")
            .await
            .unwrap()
            .is_some());
        assert!(db
            .find_direct_asset_for_key(&restricted_key.id, "second")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            db.find_direct_asset_for_key(&restricted_key.id, "shared.internal")
                .await
                .unwrap()
                .unwrap()
                .id,
            first.id
        );
        assert!(db
            .find_proxy_asset_for_key(&restricted_key.id, "second.hop", 22)
            .await
            .unwrap()
            .is_none());
        assert!(db
            .find_proxy_asset_for_key(&restricted_key.id, "shared.internal", 2222)
            .await
            .unwrap()
            .is_none());
        assert!(db
            .find_proxy_asset_for_key(&restricted_key.id, "shared.internal", 22)
            .await
            .unwrap()
            .is_some());
        assert!(!db
            .key_can_access_asset(&restricted_key.id, &second.id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn asset_access_restricted_empty_and_inactive_keys_have_no_access() {
        let db = HopDb::in_memory().await.unwrap();
        let key = add_test_key(&db, "restricted").await;
        let asset = db
            .add_asset(NewAsset::new("first", "10.0.0.1", 22))
            .await
            .unwrap();
        db.set_authorized_key_access(&key.id, AssetAccessMode::Restricted, &[])
            .await
            .unwrap();
        assert!(db.list_assets_for_key(&key.id).await.unwrap().is_empty());

        db.set_authorized_key_access(&key.id, AssetAccessMode::All, &[])
            .await
            .unwrap();
        db.set_authorized_key_active(&key.id, false).await.unwrap();
        assert!(db.list_assets_for_key(&key.id).await.unwrap().is_empty());
        assert!(!db.key_can_access_asset(&key.id, &asset.id).await.unwrap());
        assert!(db
            .get_active_authorized_key_by_fingerprint(&key.fingerprint)
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
    async fn asset_protocol_defaults_to_ssh_in_database_rows() {
        let db = HopDb::in_memory().await.unwrap();

        let inserted = db
            .add_asset(NewAsset::new("web-prod-01", "10.0.1.10", 22))
            .await
            .unwrap();

        assert_eq!(inserted.protocol, "ssh");
        assert_eq!(
            db.get_asset_by_name("web-prod-01")
                .await
                .unwrap()
                .unwrap()
                .protocol,
            "ssh"
        );
    }

    #[tokio::test]
    async fn non_ssh_assets_are_proxy_only_even_when_credential_is_submitted() {
        let db = HopDb::in_memory().await.unwrap();
        let credential = db
            .add_credential(NewCredential {
                id: Some("cred-1".to_string()),
                name: "windows admin".to_string(),
                username: "administrator".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("encrypted-password".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset = NewAsset::new("win-rdp", "10.0.2.20", 3389);
        asset.protocol = "rdp".to_string();
        asset.credential_id = Some(credential.id);

        let inserted = db.add_asset(asset).await.unwrap();

        assert_eq!(inserted.protocol, "tcp");
        assert_eq!(inserted.preset.as_deref(), Some("rdp"));
        assert!(inserted.credential_id.is_none());
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

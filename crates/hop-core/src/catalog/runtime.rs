use std::collections::BTreeSet;

use sqlx::{Sqlite, Transaction};

use super::Catalog;
use crate::{
    models::{AssetRow, AuthorizedKeyRow},
    new_id, normalize_asset_protocol, protocol_supports_managed_credentials,
    validate_credential_material, validate_tcp_port, Asset, AssetAccessMode, AuthorizedKey,
    Credential, HopCoreError, KnownHost, NewAsset, NewAuthorizedKey, NewCredential, NewKnownHost,
    NewSession, Result, Session,
};

impl Catalog {
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_asset_health_success(
        &self,
        asset_id: &str,
        latency_ms: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO asset_health
                (asset_id, status, checked_at, last_success_at, latency_ms, error_code, error_message)
            VALUES (?1, 'healthy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, ?2, NULL, NULL)
            ON CONFLICT(asset_id) DO UPDATE SET
                status = 'healthy', checked_at = CURRENT_TIMESTAMP,
                last_success_at = CURRENT_TIMESTAMP, latency_ms = excluded.latency_ms,
                error_code = NULL, error_message = NULL
            "#,
        )
        .bind(asset_id)
        .bind(latency_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_asset_health_failure(
        &self,
        asset_id: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO asset_health
                (asset_id, status, checked_at, error_code, error_message)
            VALUES (?1, 'failed', CURRENT_TIMESTAMP, ?2, ?3)
            ON CONFLICT(asset_id) DO UPDATE SET
                status = 'failed', checked_at = CURRENT_TIMESTAMP, latency_ms = NULL,
                error_code = excluded.error_code, error_message = excluded.error_message
            "#,
        )
        .bind(asset_id)
        .bind(error_code)
        .bind(error_message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_authorized_key(&self, key: NewAuthorizedKey) -> Result<AuthorizedKey> {
        self.add_authorized_key_with_access(key, AssetAccessMode::All, &[])
            .await
    }

    pub async fn add_authorized_key_with_access(
        &self,
        key: NewAuthorizedKey,
        mode: AssetAccessMode,
        asset_ids: &[String],
    ) -> Result<AuthorizedKey> {
        let id = new_id();
        let mut transaction = self.pool.begin().await?;
        validate_asset_ids(&mut transaction, asset_ids).await?;
        sqlx::query(
            r#"
            INSERT INTO access_keys (id, name, public_key, fingerprint, enabled, access_mode)
            VALUES (?1, ?2, ?3, ?4, 1, ?5)
            "#,
        )
        .bind(&id)
        .bind(key.name)
        .bind(key.public_key)
        .bind(key.fingerprint)
        .bind(mode.as_str())
        .execute(&mut *transaction)
        .await?;
        insert_local_ownership(&mut transaction, "access_key", &id).await?;
        replace_access_assignments(&mut transaction, &id, asset_ids).await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        self.get_authorized_key_by_id(&id)
            .await?
            .ok_or_else(row_not_found)
    }

    pub async fn list_authorized_keys(&self) -> Result<Vec<AuthorizedKey>> {
        let rows = sqlx::query_as::<_, AuthorizedKeyRow>(
            r#"
            SELECT id, name, public_key, fingerprint, enabled AS is_active,
                   access_mode AS asset_access_mode, created_at
            FROM access_keys
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
            SELECT id, name, public_key, fingerprint, enabled AS is_active,
                   access_mode AS asset_access_mode, created_at
            FROM access_keys
            WHERE fingerprint = ?1 AND enabled = 1
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
            SELECT id, name, public_key, fingerprint, enabled AS is_active,
                   access_mode AS asset_access_mode, created_at
            FROM access_keys
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(AuthorizedKey::try_from).transpose()
    }

    pub async fn set_authorized_key_active(&self, id: &str, active: bool) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        ensure_local(&mut transaction, "access_key", id).await?;
        let changed = sqlx::query("UPDATE access_keys SET enabled = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2 AND enabled != ?1")
            .bind(active)
            .bind(id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        if changed > 0 {
            increment_revision(&mut transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn set_authorized_key_access(
        &self,
        id: &str,
        mode: AssetAccessMode,
        asset_ids: &[String],
    ) -> Result<()> {
        if mode == AssetAccessMode::All && !asset_ids.is_empty() {
            return Err(HopCoreError::Validation(
                "all access mode cannot contain asset assignments".to_string(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        ensure_local(&mut transaction, "access_key", id).await?;
        validate_asset_ids(&mut transaction, asset_ids).await?;
        sqlx::query(
            "UPDATE access_keys SET access_mode = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        )
        .bind(mode.as_str())
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        replace_access_assignments(&mut transaction, id, asset_ids).await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_asset_ids_for_key(&self, key_id: &str) -> Result<Vec<String>> {
        sqlx::query_scalar(
            "SELECT asset_id FROM access_key_assets WHERE key_id = ?1 ORDER BY asset_id",
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_authorized_key(&self, id: &str) -> Result<()> {
        delete_local_resource(self, "access_key", "access_keys", id).await
    }

    pub async fn add_credential(&self, credential: NewCredential) -> Result<Credential> {
        validate_credential_material(
            &credential.auth_type,
            credential.password_enc.as_deref(),
            credential.private_key_enc.as_deref(),
            credential.passphrase_enc.as_deref(),
        )?;
        let id = credential.id.unwrap_or_else(new_id);
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO credentials
                (id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&id)
        .bind(credential.name)
        .bind(credential.username)
        .bind(local_auth_type(&credential.auth_type))
        .bind(credential.password_enc)
        .bind(credential.private_key_enc)
        .bind(credential.passphrase_enc)
        .execute(&mut *transaction)
        .await?;
        insert_local_ownership(&mut transaction, "credential", &id).await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        self.get_credential(&id).await?.ok_or_else(row_not_found)
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        sqlx::query_as::<_, Credential>(&credential_select("ORDER BY created_at DESC, name ASC"))
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        sqlx::query_as::<_, Credential>(&credential_select("WHERE id = ?1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn update_credential(&self, id: &str, credential: NewCredential) -> Result<()> {
        validate_credential_material(
            &credential.auth_type,
            credential.password_enc.as_deref(),
            credential.private_key_enc.as_deref(),
            credential.passphrase_enc.as_deref(),
        )?;
        let mut transaction = self.pool.begin().await?;
        ensure_local(&mut transaction, "credential", id).await?;
        sqlx::query(
            r#"
            UPDATE credentials
            SET name = ?1, username = ?2, auth_type = ?3, password_enc = ?4,
                private_key_enc = ?5, passphrase_enc = ?6, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?7
            "#,
        )
        .bind(credential.name)
        .bind(credential.username)
        .bind(local_auth_type(&credential.auth_type))
        .bind(credential.password_enc)
        .bind(credential.private_key_enc)
        .bind(credential.passphrase_enc)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn delete_credential(&self, id: &str) -> Result<()> {
        delete_local_resource(self, "credential", "credentials", id).await
    }

    pub async fn add_asset(&self, asset: NewAsset) -> Result<Asset> {
        let id = new_id();
        let mut transaction = self.pool.begin().await?;
        write_asset(&mut transaction, &id, asset, false).await?;
        insert_local_ownership(&mut transaction, "asset", &id).await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        self.get_asset_by_id(&id).await?.ok_or_else(row_not_found)
    }

    pub async fn update_asset(&self, id: &str, asset: NewAsset) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        ensure_local(&mut transaction, "asset", id).await?;
        write_asset(&mut transaction, id, asset, true).await?;
        increment_revision(&mut transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn delete_asset(&self, id: &str) -> Result<()> {
        delete_local_resource(self, "asset", "assets", id).await
    }

    pub async fn list_assets(&self) -> Result<Vec<Asset>> {
        asset_rows(
            sqlx::query_as::<_, AssetRow>(&asset_select("ORDER BY name ASC"))
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_asset_by_id(&self, id: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(&asset_select("WHERE id = ?1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn get_asset_by_name(&self, name: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(&asset_select("WHERE name = ?1"))
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn list_assets_for_key(&self, key_id: &str) -> Result<Vec<Asset>> {
        let rows = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.id, a.name, a.asset_type AS protocol, a.preset, a.host AS hostname,
                   a.port, a.description, a.tags_json AS tags, a.credential_id,
                   a.created_at, a.updated_at
            FROM assets a
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE k.access_mode = 'all'
               OR EXISTS (SELECT 1 FROM access_key_assets aka
                          WHERE aka.key_id = k.id AND aka.asset_id = a.id)
            ORDER BY a.name ASC
            "#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await?;
        asset_rows(rows)
    }

    pub async fn find_direct_asset_for_key(
        &self,
        key_id: &str,
        target: &str,
    ) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.id, a.name, a.asset_type AS protocol, a.preset, a.host AS hostname,
                   a.port, a.description, a.tags_json AS tags, a.credential_id,
                   a.created_at, a.updated_at
            FROM assets a
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE (k.access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM access_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id))
              AND (a.name = ?2 OR a.host = ?2)
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
                SELECT 1 FROM access_keys k JOIN assets a ON a.id = ?2
                WHERE k.id = ?1 AND k.enabled = 1
                  AND (k.access_mode = 'all' OR EXISTS (
                        SELECT 1 FROM access_key_assets aka
                        WHERE aka.key_id = k.id AND aka.asset_id = a.id)))
            "#,
        )
        .bind(key_id)
        .bind(asset_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
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
            SELECT a.id, a.name, a.asset_type AS protocol, a.preset, a.host AS hostname,
                   a.port, a.description, a.tags_json AS tags, a.credential_id,
                   a.created_at, a.updated_at
            FROM assets a
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE (k.access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM access_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id))
              AND ((a.host = ?2 AND a.port = ?3) OR a.name = ?4)
            ORDER BY CASE WHEN a.host = ?2 AND a.port = ?3 THEN 0 ELSE 1 END, a.name ASC
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
            INSERT INTO sessions
                (id, key_finger, key_name, mode, asset_name, target_host, target_port, client_ip, status)
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
        self.get_session(&id).await?.ok_or_else(row_not_found)
    }

    pub async fn finish_session(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?1, error = ?2, ended_at = CURRENT_TIMESTAMP WHERE id = ?3")
            .bind(status)
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        sqlx::query_as::<_, Session>(&session_select("WHERE id = ?1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<Session>> {
        sqlx::query_as::<_, Session>(&session_select("ORDER BY started_at DESC LIMIT ?1"))
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Remove completed session records older than the configured retention
    /// window. A value of zero disables automatic cleanup.
    pub async fn prune_finished_sessions(&self, retention_days: u32) -> Result<u64> {
        if retention_days == 0 {
            return Ok(0);
        }
        let result = sqlx::query(
            "DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < datetime('now', '-' || ?1 || ' days')",
        )
        .bind(i64::from(retention_days))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn get_known_host(
        &self,
        hostname: &str,
        port: i64,
        key_type: &str,
    ) -> Result<Option<KnownHost>> {
        sqlx::query_as::<_, KnownHost>(
            "SELECT hostname, port, key_type, fingerprint, first_seen FROM known_hosts WHERE hostname = ?1 AND port = ?2 AND key_type = ?3",
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
}

fn asset_select(suffix: &str) -> String {
    format!(
        "SELECT id, name, asset_type AS protocol, preset, host AS hostname, port, description, tags_json AS tags, credential_id, created_at, updated_at FROM assets {suffix}"
    )
}

fn credential_select(suffix: &str) -> String {
    format!(
        "SELECT id, name, username, CASE WHEN auth_type = 'password' THEN 'password' WHEN passphrase_enc IS NULL THEN 'key' ELSE 'key+passphrase' END AS auth_type, password_enc, private_key_enc, passphrase_enc, created_at FROM credentials {suffix}"
    )
}

fn session_select(suffix: &str) -> String {
    format!(
        "SELECT id, key_finger, key_name, mode, asset_name, target_host, target_port, client_ip, status, error, started_at, ended_at FROM sessions {suffix}"
    )
}

fn asset_rows(rows: Vec<AssetRow>) -> Result<Vec<Asset>> {
    rows.into_iter().map(Asset::try_from).collect()
}

fn local_auth_type(auth_type: &crate::AuthType) -> &'static str {
    match auth_type {
        crate::AuthType::Password => "password",
        crate::AuthType::Key | crate::AuthType::KeyWithPassphrase => "ssh_key",
    }
}

async fn write_asset(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
    asset: NewAsset,
    update: bool,
) -> Result<()> {
    let (protocol, preset) = normalize_asset_protocol(&asset.protocol, asset.preset.as_deref())?;
    let port = validate_tcp_port(asset.port)?;
    let tags = serde_json::to_string(&asset.tags)?;
    let credential_id = if protocol_supports_managed_credentials(&protocol) {
        asset.credential_id
    } else {
        None
    };
    if update {
        sqlx::query(
            r#"
            UPDATE assets SET name = ?1, asset_type = ?2, preset = ?3, host = ?4,
                port = ?5, description = ?6, tags_json = ?7, credential_id = ?8,
                updated_at = CURRENT_TIMESTAMP WHERE id = ?9
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
        .execute(&mut **transaction)
        .await?;
    } else {
        sqlx::query(
            r#"
            INSERT INTO assets
                (id, name, asset_type, preset, host, port, description, tags_json, credential_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(asset.name)
        .bind(protocol)
        .bind(preset)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(asset.description)
        .bind(tags)
        .bind(credential_id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn validate_asset_ids(
    transaction: &mut Transaction<'_, Sqlite>,
    asset_ids: &[String],
) -> Result<()> {
    for asset_id in BTreeSet::from_iter(asset_ids.iter().map(String::as_str)) {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM assets WHERE id = ?1)")
            .bind(asset_id)
            .fetch_one(&mut **transaction)
            .await?;
        if !exists {
            return Err(HopCoreError::Validation(format!(
                "unknown asset id: {asset_id}"
            )));
        }
    }
    Ok(())
}

async fn replace_access_assignments(
    transaction: &mut Transaction<'_, Sqlite>,
    key_id: &str,
    asset_ids: &[String],
) -> Result<()> {
    sqlx::query("DELETE FROM access_key_assets WHERE key_id = ?1")
        .bind(key_id)
        .execute(&mut **transaction)
        .await?;
    for asset_id in BTreeSet::from_iter(asset_ids.iter().map(String::as_str)) {
        sqlx::query("INSERT INTO access_key_assets (key_id, asset_id) VALUES (?1, ?2)")
            .bind(key_id)
            .bind(asset_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn insert_local_ownership(
    transaction: &mut Transaction<'_, Sqlite>,
    resource_type: &str,
    resource_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO resource_ownership (resource_type, resource_id, management_mode) VALUES (?1, ?2, 'local')",
    )
    .bind(resource_type)
    .bind(resource_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn ensure_local(
    transaction: &mut Transaction<'_, Sqlite>,
    resource_type: &str,
    resource_id: &str,
) -> Result<()> {
    let owner = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT management_mode, source_id FROM resource_ownership WHERE resource_type = ?1 AND resource_id = ?2",
    )
    .bind(resource_type)
    .bind(resource_id)
    .fetch_optional(&mut **transaction)
    .await?;
    match owner {
        Some((mode, _)) if mode == "local" => Ok(()),
        Some((_, source)) => Err(HopCoreError::Validation(format!(
            "managed_by_source: resource is managed by {}",
            source.as_deref().unwrap_or("a declarative source")
        ))),
        None => Err(HopCoreError::Validation(format!(
            "unknown {resource_type} id: {resource_id}"
        ))),
    }
}

async fn increment_revision(transaction: &mut Transaction<'_, Sqlite>) -> Result<()> {
    sqlx::query("UPDATE catalog_meta SET revision = revision + 1 WHERE singleton_id = 1")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn delete_local_resource(
    catalog: &Catalog,
    resource_type: &str,
    table: &str,
    id: &str,
) -> Result<()> {
    let mut transaction = catalog.pool.begin().await?;
    ensure_local(&mut transaction, resource_type, id).await?;
    sqlx::query("DELETE FROM resource_ownership WHERE resource_type = ?1 AND resource_id = ?2")
        .bind(resource_type)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    let query = match table {
        "access_keys" => "DELETE FROM access_keys WHERE id = ?1",
        "credentials" => "DELETE FROM credentials WHERE id = ?1",
        "assets" => "DELETE FROM assets WHERE id = ?1",
        _ => {
            return Err(HopCoreError::Validation(
                "invalid catalog table".to_string(),
            ))
        }
    };
    sqlx::query(query)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    increment_revision(&mut transaction).await?;
    transaction.commit().await?;
    Ok(())
}

fn row_not_found() -> HopCoreError {
    HopCoreError::Database(sqlx::Error::RowNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthType, NewCredential, ASSET_PRESET_POSTGRES, ASSET_PRESET_RDP, ASSET_PROTOCOL_TCP,
    };

    #[tokio::test]
    async fn local_crud_uses_catalog_revision_and_ownership() {
        let catalog = Catalog::in_memory().await.unwrap();
        let credential = catalog
            .add_credential(NewCredential {
                id: None,
                name: "root".to_string(),
                username: "root".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("encrypted".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let asset = catalog
            .add_asset(NewAsset::new("server", "192.0.2.10", 22))
            .await
            .unwrap();
        let key = catalog
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA",
                "SHA256:test",
            ))
            .await
            .unwrap();
        catalog
            .set_authorized_key_access(
                &key.id,
                AssetAccessMode::Restricted,
                std::slice::from_ref(&asset.id),
            )
            .await
            .unwrap();

        assert_eq!(catalog.revision().await.unwrap(), 4);
        assert!(catalog
            .key_can_access_asset(&key.id, &asset.id)
            .await
            .unwrap());
        assert_eq!(catalog.list_assets_for_key(&key.id).await.unwrap().len(), 1);
        assert_eq!(
            catalog
                .get_credential(&credential.id)
                .await
                .unwrap()
                .unwrap()
                .auth_type,
            "password"
        );
    }

    #[tokio::test]
    async fn access_scope_is_identical_for_discovery_direct_managed_and_tcp_presets() {
        let catalog = Catalog::in_memory().await.unwrap();
        let ssh = catalog
            .add_asset(NewAsset::new("shell", "192.0.2.10", 22))
            .await
            .unwrap();
        let rdp = catalog
            .add_asset(NewAsset {
                name: "desktop".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                preset: Some(ASSET_PRESET_RDP.to_string()),
                hostname: "192.0.2.20".to_string(),
                port: 3389,
                description: None,
                tags: Vec::new(),
                credential_id: None,
            })
            .await
            .unwrap();
        let database = catalog
            .add_asset(NewAsset {
                name: "postgres".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                preset: Some(ASSET_PRESET_POSTGRES.to_string()),
                hostname: "192.0.2.30".to_string(),
                port: 5432,
                description: None,
                tags: Vec::new(),
                credential_id: None,
            })
            .await
            .unwrap();
        let all = catalog
            .add_authorized_key(NewAuthorizedKey::new(
                "all",
                "ssh-ed25519 AAAA-all",
                "SHA256:all",
            ))
            .await
            .unwrap();
        let restricted = catalog
            .add_authorized_key_with_access(
                NewAuthorizedKey::new(
                    "restricted",
                    "ssh-ed25519 AAAA-restricted",
                    "SHA256:restricted",
                ),
                AssetAccessMode::Restricted,
                &[ssh.id.clone(), rdp.id.clone()],
            )
            .await
            .unwrap();
        let empty = catalog
            .add_authorized_key_with_access(
                NewAuthorizedKey::new("empty", "ssh-ed25519 AAAA-empty", "SHA256:empty"),
                AssetAccessMode::Restricted,
                &[],
            )
            .await
            .unwrap();
        let disabled = catalog
            .add_authorized_key(NewAuthorizedKey::new(
                "disabled",
                "ssh-ed25519 AAAA-disabled",
                "SHA256:disabled",
            ))
            .await
            .unwrap();
        catalog
            .set_authorized_key_active(&disabled.id, false)
            .await
            .unwrap();

        assert_eq!(catalog.list_assets_for_key(&all.id).await.unwrap().len(), 3);
        assert_eq!(
            catalog
                .list_assets_for_key(&restricted.id)
                .await
                .unwrap()
                .into_iter()
                .map(|asset| asset.id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([ssh.id.clone(), rdp.id.clone()])
        );
        assert!(catalog
            .find_direct_asset_for_key(&restricted.id, "shell")
            .await
            .unwrap()
            .is_some());
        assert!(catalog
            .find_direct_asset_for_key(&restricted.id, "postgres")
            .await
            .unwrap()
            .is_none());
        assert!(catalog
            .key_can_access_asset(&restricted.id, &ssh.id)
            .await
            .unwrap());
        assert!(!catalog
            .key_can_access_asset(&restricted.id, &database.id)
            .await
            .unwrap());
        assert!(catalog
            .find_proxy_asset_for_key(&restricted.id, "desktop.hop", 3389)
            .await
            .unwrap()
            .is_some());
        assert!(catalog
            .find_proxy_asset_for_key(&restricted.id, "192.0.2.30", 5432)
            .await
            .unwrap()
            .is_none());

        for key_id in [&empty.id, &disabled.id] {
            assert!(catalog
                .list_assets_for_key(key_id)
                .await
                .unwrap()
                .is_empty());
            assert!(catalog
                .find_direct_asset_for_key(key_id, "shell")
                .await
                .unwrap()
                .is_none());
            assert!(!catalog.key_can_access_asset(key_id, &ssh.id).await.unwrap());
            assert!(catalog
                .find_proxy_asset_for_key(key_id, "desktop.hop", 3389)
                .await
                .unwrap()
                .is_none());
        }
    }

    #[tokio::test]
    async fn finished_session_retention_preserves_active_and_recent_records() {
        let catalog = Catalog::in_memory().await.unwrap();
        let session = || NewSession {
            key_finger: "SHA256:test".to_string(),
            key_name: Some("laptop".to_string()),
            mode: "direct".to_string(),
            asset_name: Some("server".to_string()),
            target_host: Some("192.0.2.10".to_string()),
            target_port: Some(22),
            client_ip: Some("127.0.0.1".to_string()),
        };
        let expired = catalog.start_session(session()).await.unwrap();
        catalog
            .finish_session(&expired.id, "ok", None)
            .await
            .unwrap();
        sqlx::query("UPDATE sessions SET ended_at = datetime('now', '-31 days') WHERE id = ?1")
            .bind(&expired.id)
            .execute(catalog.pool())
            .await
            .unwrap();
        let recent = catalog.start_session(session()).await.unwrap();
        catalog
            .finish_session(&recent.id, "ok", None)
            .await
            .unwrap();
        let active = catalog.start_session(session()).await.unwrap();

        assert_eq!(catalog.prune_finished_sessions(0).await.unwrap(), 0);
        assert_eq!(catalog.prune_finished_sessions(30).await.unwrap(), 1);
        assert!(catalog.get_session(&expired.id).await.unwrap().is_none());
        assert!(catalog.get_session(&recent.id).await.unwrap().is_some());
        assert!(catalog.get_session(&active.id).await.unwrap().is_some());
    }
}

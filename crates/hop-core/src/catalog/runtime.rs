use std::collections::{BTreeMap, BTreeSet};

use sqlx::{Sqlite, Transaction};

use super::Catalog;
use crate::{
    models::{AssetRow, AuthorizedKeyRow},
    new_id, protocol_supports_managed_credentials, validate_asset_protocol,
    validate_credential_material, validate_tcp_port, Asset, AssetAccessMode, AssetProbeSample,
    AuditEvent, AuthorizedKey, Credential, HopCoreError, KnownHost, NewAsset, NewAuditEvent,
    NewAuthorizedKey, NewCredential, NewKnownHost, NewNotificationChannel, NewSession,
    NotificationChannel, NotificationChannelKind, ProbeSettings, Result, Session,
    ASSET_HEALTH_FAILED, ASSET_HEALTH_HEALTHY, ASSET_PROTOCOL_SSH, MAX_ASSET_CHAIN_HOPS,
    MAX_PROBE_SAMPLES_PER_ASSET,
};

impl Catalog {
    pub async fn resource_ownership(
        &self,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<super::ResourceOwnership> {
        let mode = sqlx::query_scalar::<_, String>(
            "SELECT management_mode FROM resource_ownership WHERE resource_type = ?1 AND resource_id = ?2",
        )
        .bind(resource_type)
        .bind(resource_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match mode.as_deref() {
            Some("declarative") => super::ResourceOwnership::Config,
            _ => super::ResourceOwnership::Local,
        })
    }

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
                (asset_id, status, checked_at, last_success_at, latency_ms, error_code, error_message, consecutive_failures)
            VALUES (?1, 'healthy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, ?2, NULL, NULL, 0)
            ON CONFLICT(asset_id) DO UPDATE SET
                status = 'healthy', checked_at = CURRENT_TIMESTAMP,
                last_success_at = CURRENT_TIMESTAMP, latency_ms = excluded.latency_ms,
                error_code = NULL, error_message = NULL, consecutive_failures = 0
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
                (asset_id, status, checked_at, error_code, error_message, consecutive_failures)
            VALUES (?1, 'failed', CURRENT_TIMESTAMP, ?2, ?3, 1)
            ON CONFLICT(asset_id) DO UPDATE SET
                status = 'failed', checked_at = CURRENT_TIMESTAMP, latency_ms = NULL,
                error_code = excluded.error_code, error_message = excluded.error_message,
                consecutive_failures = COALESCE(asset_health.consecutive_failures, 0) + 1
            "#,
        )
        .bind(asset_id)
        .bind(error_code)
        .bind(error_message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_probe_sample(
        &self,
        asset_id: &str,
        healthy: bool,
        latency_ms: Option<i64>,
        error_code: Option<&str>,
    ) -> Result<()> {
        let status = if healthy {
            ASSET_HEALTH_HEALTHY
        } else {
            ASSET_HEALTH_FAILED
        };
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO asset_probe_samples (asset_id, status, latency_ms, error_code)
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(asset_id)
        .bind(status)
        .bind(latency_ms)
        .bind(error_code)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            r#"
            DELETE FROM asset_probe_samples
            WHERE asset_id = ?1
              AND id NOT IN (
                  SELECT id FROM asset_probe_samples
                  WHERE asset_id = ?1
                  ORDER BY id DESC
                  LIMIT ?2
              )
            "#,
        )
        .bind(asset_id)
        .bind(MAX_PROBE_SAMPLES_PER_ASSET)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn mark_asset_alert_firing(&self, asset_id: &str, firing: bool) -> Result<()> {
        if firing {
            sqlx::query(
                "UPDATE asset_health SET alert_firing = 1, last_alerted_at = CURRENT_TIMESTAMP WHERE asset_id = ?1",
            )
            .bind(asset_id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE asset_health SET alert_firing = 0, last_recovered_at = CURRENT_TIMESTAMP WHERE asset_id = ?1",
            )
            .bind(asset_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn list_asset_probe_samples(
        &self,
        asset_id: &str,
        limit: i64,
    ) -> Result<Vec<AssetProbeSample>> {
        if !self.has_v0_3_schema() {
            return Ok(Vec::new());
        }
        let bounded = limit.clamp(1, MAX_PROBE_SAMPLES_PER_ASSET);
        let mut samples = sqlx::query_as::<_, AssetProbeSample>(
            r#"
            SELECT id, asset_id, checked_at, status, latency_ms, error_code
            FROM asset_probe_samples
            WHERE asset_id = ?1
            ORDER BY id DESC
            LIMIT ?2
            "#,
        )
        .bind(asset_id)
        .bind(bounded)
        .fetch_all(&self.pool)
        .await?;
        samples.reverse();
        Ok(samples)
    }

    pub async fn list_recent_probe_samples(
        &self,
        per_asset_limit: i64,
    ) -> Result<BTreeMap<String, Vec<AssetProbeSample>>> {
        if !self.has_v0_3_schema() {
            return Ok(BTreeMap::new());
        }
        let bounded = per_asset_limit.clamp(1, MAX_PROBE_SAMPLES_PER_ASSET);
        let rows = sqlx::query_as::<_, AssetProbeSample>(
            r#"
            SELECT id, asset_id, checked_at, status, latency_ms, error_code
            FROM (
                SELECT id, asset_id, checked_at, status, latency_ms, error_code,
                       ROW_NUMBER() OVER (PARTITION BY asset_id ORDER BY id DESC) AS rn
                FROM asset_probe_samples
            )
            WHERE rn <= ?1
            ORDER BY asset_id ASC, id ASC
            "#,
        )
        .bind(bounded)
        .fetch_all(&self.pool)
        .await?;
        let mut map: BTreeMap<String, Vec<AssetProbeSample>> = BTreeMap::new();
        for sample in rows {
            map.entry(sample.asset_id.clone()).or_default().push(sample);
        }
        Ok(map)
    }

    pub async fn get_probe_settings(&self) -> Result<ProbeSettings> {
        if !self.has_v0_3_schema() {
            return Ok(ProbeSettings::default());
        }
        let settings = sqlx::query_as::<_, ProbeSettings>(
            r#"
            SELECT enabled, interval_seconds, timeout_ms, failure_threshold, notify_on_recovery, updated_at
            FROM probe_settings
            WHERE singleton_id = 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(settings.unwrap_or_default())
    }

    pub async fn update_probe_settings(&self, settings: &ProbeSettings) -> Result<ProbeSettings> {
        if !(30..=86_400).contains(&settings.interval_seconds) {
            return Err(HopCoreError::Validation(
                "probe interval_seconds must be between 30 and 86400".to_string(),
            ));
        }
        if !(500..=15_000).contains(&settings.timeout_ms) {
            return Err(HopCoreError::Validation(
                "probe timeout_ms must be between 500 and 15000".to_string(),
            ));
        }
        if !(1..=10).contains(&settings.failure_threshold) {
            return Err(HopCoreError::Validation(
                "probe failure_threshold must be between 1 and 10".to_string(),
            ));
        }
        sqlx::query(
            r#"
            INSERT INTO probe_settings
                (singleton_id, enabled, interval_seconds, timeout_ms, failure_threshold, notify_on_recovery, updated_at)
            VALUES (1, ?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
            ON CONFLICT(singleton_id) DO UPDATE SET
                enabled = excluded.enabled,
                interval_seconds = excluded.interval_seconds,
                timeout_ms = excluded.timeout_ms,
                failure_threshold = excluded.failure_threshold,
                notify_on_recovery = excluded.notify_on_recovery,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(settings.enabled)
        .bind(settings.interval_seconds)
        .bind(settings.timeout_ms)
        .bind(settings.failure_threshold)
        .bind(settings.notify_on_recovery)
        .execute(&self.pool)
        .await?;
        self.get_probe_settings().await
    }

    pub async fn list_notification_channels(&self) -> Result<Vec<NotificationChannel>> {
        if !self.has_v0_3_schema() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, NotificationChannel>(
            r#"
            SELECT id, name, kind, enabled, config_enc, created_at, updated_at
            FROM notification_channels
            ORDER BY created_at ASC, name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_notification_channel(&self, id: &str) -> Result<Option<NotificationChannel>> {
        if !self.has_v0_3_schema() {
            return Ok(None);
        }
        sqlx::query_as::<_, NotificationChannel>(
            r#"
            SELECT id, name, kind, enabled, config_enc, created_at, updated_at
            FROM notification_channels
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn add_notification_channel(
        &self,
        channel: NewNotificationChannel,
    ) -> Result<NotificationChannel> {
        let name = channel.name.trim().to_string();
        if name.is_empty() {
            return Err(HopCoreError::Validation(
                "notification channel name must not be empty".to_string(),
            ));
        }
        if channel.config_enc.trim().is_empty() {
            return Err(HopCoreError::Validation(
                "notification channel config_enc must not be empty".to_string(),
            ));
        }
        let _ = NotificationChannelKind::try_from(channel.kind.as_str())?;
        let id = channel.id.unwrap_or_else(new_id);
        sqlx::query(
            r#"
            INSERT INTO notification_channels (id, name, kind, enabled, config_enc)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(&id)
        .bind(&name)
        .bind(channel.kind.as_str())
        .bind(channel.enabled)
        .bind(&channel.config_enc)
        .execute(&self.pool)
        .await?;
        self.get_notification_channel(&id)
            .await?
            .ok_or_else(row_not_found)
    }

    pub async fn update_notification_channel(
        &self,
        id: &str,
        channel: NewNotificationChannel,
    ) -> Result<NotificationChannel> {
        let name = channel.name.trim().to_string();
        if name.is_empty() {
            return Err(HopCoreError::Validation(
                "notification channel name must not be empty".to_string(),
            ));
        }
        if channel.config_enc.trim().is_empty() {
            return Err(HopCoreError::Validation(
                "notification channel config_enc must not be empty".to_string(),
            ));
        }
        let rows = sqlx::query(
            r#"
            UPDATE notification_channels
            SET name = ?1, kind = ?2, enabled = ?3, config_enc = ?4, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?5
            "#,
        )
        .bind(&name)
        .bind(channel.kind.as_str())
        .bind(channel.enabled)
        .bind(&channel.config_enc)
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if rows == 0 {
            return Err(row_not_found());
        }
        self.get_notification_channel(id)
            .await?
            .ok_or_else(row_not_found)
    }

    pub async fn delete_notification_channel(&self, id: &str) -> Result<bool> {
        let rows = sqlx::query("DELETE FROM notification_channels WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(rows > 0)
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

    pub async fn resolve_asset_chain(&self, asset_id: &str) -> Result<Vec<Asset>> {
        let mut chain = Vec::with_capacity(2);
        let mut visited = BTreeSet::new();
        let mut current_id = asset_id.to_string();

        loop {
            if !visited.insert(current_id.clone()) {
                return Err(HopCoreError::Validation(format!(
                    "via_asset cycle detected while resolving asset {asset_id}"
                )));
            }
            let asset = self.get_asset_by_id(&current_id).await?.ok_or_else(|| {
                HopCoreError::Validation(format!("unknown asset id: {current_id}"))
            })?;
            let is_intermediate = !chain.is_empty();
            if is_intermediate {
                if asset.protocol != ASSET_PROTOCOL_SSH || asset.credential_id.is_none() {
                    return Err(HopCoreError::Validation(format!(
                        "via_asset {} must be a managed ssh asset with a credential",
                        asset.name
                    )));
                }
                if chain.len() > MAX_ASSET_CHAIN_HOPS {
                    return Err(HopCoreError::Validation(format!(
                        "via_asset chain exceeds maximum depth of {MAX_ASSET_CHAIN_HOPS} hops"
                    )));
                }
            }
            let next_via = asset.via_asset_id.clone();
            chain.push(asset);
            match next_via {
                Some(via_id) => current_id = via_id,
                None => break,
            }
        }

        chain.reverse();
        Ok(chain)
    }

    /// Atomically creates a new SSH-key credential, binds a locally managed SSH
    /// asset to it, and optionally deletes the asset's previous local credential
    /// when no other asset references it.
    pub async fn bootstrap_bind_asset_credential(
        &self,
        asset_id: &str,
        credential: NewCredential,
        prune_previous_unused_credential: bool,
    ) -> Result<(Asset, Credential, bool)> {
        validate_credential_material(
            &credential.auth_type,
            credential.password_enc.as_deref(),
            credential.private_key_enc.as_deref(),
            credential.passphrase_enc.as_deref(),
        )?;
        let cred_id = credential.id.unwrap_or_else(new_id);
        let mut transaction = self.pool.begin().await?;
        ensure_local(&mut transaction, "asset", asset_id).await?;

        let existing = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT asset_type, credential_id FROM assets WHERE id = ?1",
        )
        .bind(asset_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(row_not_found)?;

        if existing.0 != ASSET_PROTOCOL_SSH {
            return Err(HopCoreError::Validation(
                "only ssh assets support SSH key bootstrap".to_string(),
            ));
        }
        let previous_credential_id = existing.1;

        sqlx::query(
            r#"
            INSERT INTO credentials
                (id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&cred_id)
        .bind(&credential.name)
        .bind(&credential.username)
        .bind(local_auth_type(&credential.auth_type))
        .bind(&credential.password_enc)
        .bind(&credential.private_key_enc)
        .bind(&credential.passphrase_enc)
        .execute(&mut *transaction)
        .await?;
        insert_local_ownership(&mut transaction, "credential", &cred_id).await?;

        sqlx::query(
            "UPDATE assets SET credential_id = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        )
        .bind(&cred_id)
        .bind(asset_id)
        .execute(&mut *transaction)
        .await?;

        let mut pruned_previous = false;
        if prune_previous_unused_credential {
            if let Some(prev_id) = previous_credential_id.as_deref() {
                if prev_id != cred_id {
                    let still_used: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM assets WHERE credential_id = ?1)",
                    )
                    .bind(prev_id)
                    .fetch_one(&mut *transaction)
                    .await?;
                    let prev_local = sqlx::query_scalar::<_, String>(
                        "SELECT management_mode FROM resource_ownership WHERE resource_type = 'credential' AND resource_id = ?1",
                    )
                    .bind(prev_id)
                    .fetch_optional(&mut *transaction)
                    .await?;
                    if !still_used && prev_local.as_deref() == Some("local") {
                        sqlx::query(
                            "DELETE FROM resource_ownership WHERE resource_type = 'credential' AND resource_id = ?1",
                        )
                        .bind(prev_id)
                        .execute(&mut *transaction)
                        .await?;
                        sqlx::query("DELETE FROM credentials WHERE id = ?1")
                            .bind(prev_id)
                            .execute(&mut *transaction)
                            .await?;
                        pruned_previous = true;
                    }
                }
            }
        }

        increment_revision(&mut transaction).await?;
        transaction.commit().await?;

        let updated_asset = self
            .get_asset_by_id(asset_id)
            .await?
            .ok_or_else(row_not_found)?;
        let saved_cred = self
            .get_credential(&cred_id)
            .await?
            .ok_or_else(row_not_found)?;
        Ok((updated_asset, saved_cred, pruned_previous))
    }

    pub async fn list_assets(&self) -> Result<Vec<Asset>> {
        asset_rows(
            sqlx::query_as::<_, AssetRow>(&asset_select(
                self.has_v0_3_schema(),
                "ORDER BY a.name ASC",
            ))
            .fetch_all(&self.pool)
            .await?,
        )
    }

    pub async fn get_asset_by_id(&self, id: &str) -> Result<Option<Asset>> {
        let row =
            sqlx::query_as::<_, AssetRow>(&asset_select(self.has_v0_3_schema(), "WHERE a.id = ?1"))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn get_asset_by_name(&self, name: &str) -> Result<Option<Asset>> {
        let row = sqlx::query_as::<_, AssetRow>(&asset_select(
            self.has_v0_3_schema(),
            "WHERE a.name = ?1",
        ))
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Asset::try_from).transpose()
    }

    pub async fn list_assets_for_key(&self, key_id: &str) -> Result<Vec<Asset>> {
        let rows = sqlx::query_as::<_, AssetRow>(&asset_select(
            self.has_v0_3_schema(),
            r#"
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE k.access_mode = 'all'
               OR EXISTS (SELECT 1 FROM access_key_assets aka
                          WHERE aka.key_id = k.id AND aka.asset_id = a.id)
            ORDER BY a.name ASC
            "#,
        ))
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
        let row = sqlx::query_as::<_, AssetRow>(&asset_select(
            self.has_v0_3_schema(),
            r#"
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE (k.access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM access_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id))
              AND (a.name = ?2 OR a.host = ?2)
            ORDER BY CASE WHEN a.name = ?2 THEN 0 ELSE 1 END, a.name ASC
            LIMIT 1
            "#,
        ))
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
        let row = sqlx::query_as::<_, AssetRow>(&asset_select(
            self.has_v0_3_schema(),
            r#"
            JOIN access_keys k ON k.id = ?1 AND k.enabled = 1
            WHERE (k.access_mode = 'all' OR EXISTS (
                    SELECT 1 FROM access_key_assets aka
                    WHERE aka.key_id = k.id AND aka.asset_id = a.id))
              AND ((a.host = ?2 AND a.port = ?3) OR (a.name = ?4 AND a.port = ?3))
            ORDER BY CASE WHEN a.host = ?2 AND a.port = ?3 THEN 0 ELSE 1 END, a.name ASC
            LIMIT 1
            "#,
        ))
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
                (id, key_id, key_finger, key_name, mode, asset_id, asset_name, target_host, target_port, client_ip, status)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'started')
            "#,
        )
        .bind(&id)
        .bind(session.key_id)
        .bind(session.key_finger)
        .bind(session.key_name)
        .bind(session.mode)
        .bind(session.asset_id)
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

    /// Close session records left in the `started` state by a previous
    /// process lifetime. This is intentionally an explicit startup recovery
    /// operation: the server calls it before opening either listener, so no
    /// session created by the current process can be selected.
    pub async fn mark_started_sessions_failed(&self, error: &str) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE sessions SET status = 'failed', error = ?1, ended_at = CURRENT_TIMESTAMP WHERE status = 'started' AND ended_at IS NULL",
        )
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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

    pub async fn list_known_hosts(&self) -> Result<Vec<KnownHost>> {
        sqlx::query_as::<_, KnownHost>(
            "SELECT hostname, port, key_type, fingerprint, first_seen FROM known_hosts ORDER BY hostname ASC, port ASC, key_type ASC",
        )
        .fetch_all(&self.pool)
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

    pub async fn delete_known_host(
        &self,
        hostname: &str,
        port: i64,
        key_type: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM known_hosts WHERE hostname = ?1 AND port = ?2 AND key_type = ?3",
        )
        .bind(hostname)
        .bind(port)
        .bind(key_type)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn record_audit_event(&self, event: NewAuditEvent) -> Result<AuditEvent> {
        let id = new_id();
        sqlx::query(
            r#"
            INSERT INTO audit_events
                (id, actor_label, action, target_type, target_id, target_label, result, source_ip, details_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(&id)
        .bind(event.actor_label)
        .bind(event.action)
        .bind(event.target_type)
        .bind(event.target_id)
        .bind(event.target_label)
        .bind(event.result)
        .bind(event.source_ip)
        .bind(event.details_json)
        .execute(&self.pool)
        .await?;
        sqlx::query_as::<_, AuditEvent>(&audit_event_select("WHERE id = ?1"))
            .bind(&id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(row_not_found)
    }

    pub async fn list_audit_events(&self, limit: i64) -> Result<Vec<AuditEvent>> {
        sqlx::query_as::<_, AuditEvent>(&audit_event_select(
            "ORDER BY occurred_at DESC, rowid DESC LIMIT ?1",
        ))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Remove audit event records older than the configured retention window.
    /// A value of zero disables automatic cleanup.
    pub async fn prune_audit_events(&self, retention_days: u32) -> Result<u64> {
        if retention_days == 0 {
            return Ok(0);
        }
        let result = sqlx::query(
            "DELETE FROM audit_events WHERE occurred_at < datetime('now', '-' || ?1 || ' days')",
        )
        .bind(i64::from(retention_days))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

fn asset_select(v0_3_ready: bool, suffix: &str) -> String {
    if v0_3_ready {
        format!(
            "SELECT a.id, a.name, a.asset_type AS protocol, a.host AS hostname, a.port, a.display_name, a.description, a.tags_json AS tags, a.credential_id, a.via_asset_id, a.allow_tcp_forwarding, a.created_at, a.updated_at, COALESCE(h.status, 'unknown') AS health_status, h.checked_at AS health_checked_at, h.last_success_at AS health_last_success_at, h.latency_ms AS health_latency_ms, h.error_code AS health_error_code, h.error_message AS health_error_message, COALESCE(h.consecutive_failures, 0) AS health_consecutive_failures, COALESCE(h.alert_firing, 0) AS health_alert_firing FROM assets a LEFT JOIN asset_health h ON h.asset_id = a.id {suffix}"
        )
    } else {
        format!(
            "SELECT a.id, a.name, a.asset_type AS protocol, a.host AS hostname, a.port, a.display_name, a.description, a.tags_json AS tags, a.credential_id, NULL AS via_asset_id, 1 AS allow_tcp_forwarding, a.created_at, a.updated_at, COALESCE(h.status, 'unknown') AS health_status, h.checked_at AS health_checked_at, h.last_success_at AS health_last_success_at, h.latency_ms AS health_latency_ms, h.error_code AS health_error_code, h.error_message AS health_error_message, 0 AS health_consecutive_failures, 0 AS health_alert_firing FROM assets a LEFT JOIN asset_health h ON h.asset_id = a.id {suffix}"
        )
    }
}

fn credential_select(suffix: &str) -> String {
    format!(
        "SELECT id, name, username, CASE WHEN auth_type = 'password' THEN 'password' WHEN passphrase_enc IS NULL THEN 'key' ELSE 'key+passphrase' END AS auth_type, password_enc, private_key_enc, passphrase_enc, created_at FROM credentials {suffix}"
    )
}

fn session_select(suffix: &str) -> String {
    format!(
        "SELECT id, key_id, key_finger, key_name, mode, asset_id, asset_name, target_host, target_port, client_ip, status, error, started_at, ended_at FROM sessions {suffix}"
    )
}

fn audit_event_select(suffix: &str) -> String {
    format!(
        "SELECT id, occurred_at, actor_label, action, target_type, target_id, target_label, result, source_ip, details_json FROM audit_events {suffix}"
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

#[derive(Clone, sqlx::FromRow)]
struct TopologyNode {
    id: String,
    name: String,
    asset_type: String,
    credential_id: Option<String>,
    via_asset_id: Option<String>,
}

pub(crate) type TopologyNodeMeta = (String, String, Option<String>, Option<String>);

pub(crate) fn validate_asset_topology_nodes(
    nodes: &BTreeMap<String, TopologyNodeMeta>,
) -> Result<()> {
    for (id, (name, _, _, via_id)) in nodes {
        let Some(mut cur_via) = via_id.clone() else {
            continue;
        };
        let mut hops = 0usize;
        let mut seen = BTreeSet::from([id.clone()]);
        loop {
            hops += 1;
            if hops > MAX_ASSET_CHAIN_HOPS {
                return Err(HopCoreError::Validation(format!(
                    "asset {name}: via_asset chain exceeds maximum depth of {MAX_ASSET_CHAIN_HOPS} hops"
                )));
            }
            if !seen.insert(cur_via.clone()) {
                return Err(HopCoreError::Validation(format!(
                    "asset {name}: via_asset chain contains a cycle"
                )));
            }
            let Some((via_name, via_type, via_cred, next_via)) = nodes.get(&cur_via) else {
                return Err(HopCoreError::Validation(format!(
                    "asset {name}: unknown via_asset_id {cur_via}"
                )));
            };
            if via_type != ASSET_PROTOCOL_SSH || via_cred.is_none() {
                return Err(HopCoreError::Validation(format!(
                    "asset {name}: via_asset {via_name} must be a managed ssh asset with a credential"
                )));
            }
            match next_via {
                Some(next) => cur_via = next.clone(),
                None => break,
            }
        }
    }
    Ok(())
}

async fn validate_asset_topology_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    target_id: &str,
    target_name: &str,
    target_protocol: &str,
    target_credential_id: Option<&str>,
    target_via_asset_id: Option<&str>,
) -> Result<()> {
    let existing = sqlx::query_as::<_, TopologyNode>(
        "SELECT id, name, asset_type, credential_id, via_asset_id FROM assets",
    )
    .fetch_all(&mut **transaction)
    .await?;

    let mut graph: BTreeMap<String, (String, String, Option<String>, Option<String>)> = existing
        .into_iter()
        .map(|node| {
            (
                node.id,
                (
                    node.name,
                    node.asset_type,
                    node.credential_id,
                    node.via_asset_id,
                ),
            )
        })
        .collect();
    graph.insert(
        target_id.to_string(),
        (
            target_name.to_string(),
            target_protocol.to_string(),
            target_credential_id.map(str::to_owned),
            target_via_asset_id.map(str::to_owned),
        ),
    );
    validate_asset_topology_nodes(&graph)
}

async fn write_asset(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
    asset: NewAsset,
    update: bool,
) -> Result<()> {
    let protocol = validate_asset_protocol(&asset.protocol)?;
    let port = validate_tcp_port(asset.port)?;
    let display_name = asset
        .display_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let description = asset
        .description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let tags = serde_json::to_string(&asset.tags)?;
    let credential_id = if protocol_supports_managed_credentials(&protocol) {
        asset
            .credential_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    } else {
        None
    };
    let via_asset_id = asset
        .via_asset_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let allow_tcp_forwarding = if protocol == ASSET_PROTOCOL_SSH {
        asset.allow_tcp_forwarding
    } else {
        true
    };

    validate_asset_topology_in_tx(
        transaction,
        id,
        &asset.name,
        &protocol,
        credential_id.as_deref(),
        via_asset_id.as_deref(),
    )
    .await?;

    if update {
        sqlx::query(
            r#"
            UPDATE assets SET name = ?1, asset_type = ?2, host = ?3,
                port = ?4, display_name = ?5, description = ?6, tags_json = ?7, credential_id = ?8,
                via_asset_id = ?9, allow_tcp_forwarding = ?10,
                updated_at = CURRENT_TIMESTAMP WHERE id = ?11
            "#,
        )
        .bind(asset.name)
        .bind(protocol)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(display_name)
        .bind(description)
        .bind(tags)
        .bind(credential_id)
        .bind(via_asset_id)
        .bind(allow_tcp_forwarding)
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    } else {
        sqlx::query(
            r#"
            INSERT INTO assets
                (id, name, asset_type, host, port, display_name, description, tags_json, credential_id, via_asset_id, allow_tcp_forwarding)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
        )
        .bind(id)
        .bind(asset.name)
        .bind(protocol)
        .bind(asset.hostname)
        .bind(i64::from(port))
        .bind(display_name)
        .bind(description)
        .bind(tags)
        .bind(credential_id)
        .bind(via_asset_id)
        .bind(allow_tcp_forwarding)
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
    if table == "assets" {
        if let Some(child_name) = sqlx::query_scalar::<_, String>(
            "SELECT name FROM assets WHERE via_asset_id = ?1 ORDER BY name ASC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?
        {
            return Err(HopCoreError::Validation(format!(
                "asset is still referenced as via_asset by {child_name}"
            )));
        }
    }
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
    use crate::{AuthType, NewCredential, ASSET_PROTOCOL_TCP};

    #[tokio::test]
    async fn known_hosts_can_be_listed_and_precisely_reset() {
        let catalog = Catalog::in_memory().await.unwrap();
        for (key_type, fingerprint) in
            [("ssh-ed25519", "SHA256:ed25519"), ("ssh-rsa", "SHA256:rsa")]
        {
            catalog
                .upsert_known_host(NewKnownHost {
                    hostname: "192.0.2.10".to_string(),
                    port: 22,
                    key_type: key_type.to_string(),
                    fingerprint: fingerprint.to_string(),
                })
                .await
                .unwrap();
        }

        let hosts = catalog.list_known_hosts().await.unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].key_type, "ssh-ed25519");

        assert!(catalog
            .delete_known_host("192.0.2.10", 22, "ssh-ed25519")
            .await
            .unwrap());
        assert!(catalog
            .get_known_host("192.0.2.10", 22, "ssh-ed25519")
            .await
            .unwrap()
            .is_none());
        assert!(catalog
            .get_known_host("192.0.2.10", 22, "ssh-rsa")
            .await
            .unwrap()
            .is_some());
        assert!(!catalog
            .delete_known_host("192.0.2.10", 22, "ssh-ed25519")
            .await
            .unwrap());
    }

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
    async fn access_scope_is_identical_for_discovery_direct_managed_and_tcp_assets() {
        let catalog = Catalog::in_memory().await.unwrap();
        let ssh = catalog
            .add_asset(NewAsset::new("shell", "192.0.2.10", 22))
            .await
            .unwrap();
        let rdp = catalog
            .add_asset(NewAsset {
                name: "desktop".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                hostname: "192.0.2.20".to_string(),
                port: 3389,
                display_name: Some("HK Desktop".to_string()),
                description: None,
                tags: vec!["hk".to_string()],
                credential_id: None,
                via_asset_id: None,
                allow_tcp_forwarding: true,
            })
            .await
            .unwrap();
        let database = catalog
            .add_asset(NewAsset {
                name: "postgres".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                hostname: "192.0.2.30".to_string(),
                port: 5432,
                display_name: None,
                description: None,
                tags: Vec::new(),
                credential_id: None,
                via_asset_id: None,
                allow_tcp_forwarding: true,
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
            .find_proxy_asset_for_key(&restricted.id, "desktop.hop", 22)
            .await
            .unwrap()
            .is_none());
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
    async fn asset_health_and_audit_events_round_trip_and_prune() {
        let catalog = Catalog::in_memory().await.unwrap();
        let asset = catalog
            .add_asset(NewAsset::new("hk-edge", "192.0.2.88", 22))
            .await
            .unwrap();
        assert_eq!(asset.health.status, "unknown");
        assert_eq!(asset.health.latency_ms, None);

        catalog
            .record_asset_health_success(&asset.id, Some(14))
            .await
            .unwrap();
        let healthy = catalog.get_asset_by_id(&asset.id).await.unwrap().unwrap();
        assert_eq!(healthy.health.status, "healthy");
        assert_eq!(healthy.health.latency_ms, Some(14));
        let first_success = healthy.health.last_success_at.clone();
        assert!(first_success.is_some());

        catalog
            .record_asset_health_failure(&asset.id, Some("timeout"), Some("connect timed out"))
            .await
            .unwrap();
        let failed = catalog.get_asset_by_id(&asset.id).await.unwrap().unwrap();
        assert_eq!(failed.health.status, "failed");
        assert_eq!(failed.health.last_success_at, first_success);
        assert_eq!(failed.health.error_code.as_deref(), Some("timeout"));

        let old_event = catalog
            .record_audit_event(NewAuditEvent {
                actor_label: "local-cli".to_string(),
                action: "asset.create".to_string(),
                target_type: "asset".to_string(),
                target_id: Some(asset.id.clone()),
                target_label: Some(asset.name.clone()),
                result: "success".to_string(),
                source_ip: None,
                details_json: None,
            })
            .await
            .unwrap();
        sqlx::query(
            "UPDATE audit_events SET occurred_at = datetime('now', '-45 days') WHERE id = ?1",
        )
        .bind(&old_event.id)
        .execute(catalog.pool())
        .await
        .unwrap();
        let recent_event = catalog
            .record_audit_event(NewAuditEvent {
                actor_label: "api-token".to_string(),
                action: "asset.probe".to_string(),
                target_type: "asset".to_string(),
                target_id: Some(asset.id.clone()),
                target_label: Some(asset.name.clone()),
                result: "success".to_string(),
                source_ip: Some("127.0.0.1".to_string()),
                details_json: Some(r#"{"latency_ms":14}"#.to_string()),
            })
            .await
            .unwrap();

        assert_eq!(catalog.list_audit_events(10).await.unwrap().len(), 2);
        assert_eq!(catalog.prune_audit_events(30).await.unwrap(), 1);
        let remaining = catalog.list_audit_events(10).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, recent_event.id);
    }

    #[tokio::test]
    async fn finished_session_retention_preserves_active_and_recent_records() {
        let catalog = Catalog::in_memory().await.unwrap();
        let session = || NewSession {
            key_id: None,
            key_finger: "SHA256:test".to_string(),
            key_name: Some("laptop".to_string()),
            mode: "direct".to_string(),
            asset_id: None,
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

    #[tokio::test]
    async fn startup_recovery_marks_only_unfinished_started_sessions_failed() {
        let catalog = Catalog::in_memory().await.unwrap();
        let session = || NewSession {
            key_id: None,
            key_finger: "SHA256:test".to_string(),
            key_name: Some("laptop".to_string()),
            mode: "direct".to_string(),
            asset_id: None,
            asset_name: Some("server".to_string()),
            target_host: Some("192.0.2.10".to_string()),
            target_port: Some(22),
            client_ip: Some("127.0.0.1".to_string()),
        };

        let stale = catalog.start_session(session()).await.unwrap();
        let finished = catalog.start_session(session()).await.unwrap();
        catalog
            .finish_session(&finished.id, "ok", None)
            .await
            .unwrap();

        assert_eq!(
            catalog
                .mark_started_sessions_failed("Hop server restarted before session completed")
                .await
                .unwrap(),
            1
        );

        let stale = catalog.get_session(&stale.id).await.unwrap().unwrap();
        assert_eq!(stale.status, "failed");
        assert_eq!(
            stale.error.as_deref(),
            Some("Hop server restarted before session completed")
        );
        assert!(stale.ended_at.is_some());

        let finished = catalog.get_session(&finished.id).await.unwrap().unwrap();
        assert_eq!(finished.status, "ok");
        assert_eq!(finished.error, None);
    }

    #[tokio::test]
    async fn via_asset_validates_managed_ssh_parent_cycles_depth_and_deletion() {
        let catalog = Catalog::in_memory().await.unwrap();
        let cred = catalog
            .add_credential(NewCredential {
                id: None,
                name: "relay-cred".to_string(),
                username: "root".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("enc".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let unmanaged = catalog
            .add_asset(NewAsset::new("unmanaged-entry", "198.51.100.1", 22))
            .await
            .unwrap();
        let mut bad_leaf = NewAsset::new("bad-leaf", "10.0.0.2", 22);
        bad_leaf.via_asset_id = Some(unmanaged.id.clone());
        assert!(catalog.add_asset(bad_leaf).await.is_err());

        let mut h1_new = NewAsset::new("hop-1", "198.51.100.10", 22);
        h1_new.credential_id = Some(cred.id.clone());
        let h1 = catalog.add_asset(h1_new).await.unwrap();

        let mut h2_new = NewAsset::new("hop-2", "10.10.0.2", 22);
        h2_new.credential_id = Some(cred.id.clone());
        h2_new.via_asset_id = Some(h1.id.clone());
        let h2 = catalog.add_asset(h2_new).await.unwrap();

        let mut db_leaf_new = NewAsset {
            name: "lan-postgres".to_string(),
            protocol: ASSET_PROTOCOL_TCP.to_string(),
            hostname: "10.10.0.50".to_string(),
            port: 5432,
            display_name: None,
            description: None,
            tags: Vec::new(),
            credential_id: None,
            via_asset_id: Some(h2.id.clone()),
            allow_tcp_forwarding: true,
        };
        let db_leaf = catalog.add_asset(db_leaf_new.clone()).await.unwrap();

        let chain = catalog.resolve_asset_chain(&db_leaf.id).await.unwrap();
        assert_eq!(
            chain.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["hop-1", "hop-2", "lan-postgres"]
        );

        // Reject cycle: hop-1 -> hop-2
        let mut cycle_h1 = NewAsset::new("hop-1", "198.51.100.10", 22);
        cycle_h1.credential_id = Some(cred.id.clone());
        cycle_h1.via_asset_id = Some(h2.id.clone());
        assert!(catalog.update_asset(&h1.id, cycle_h1).await.is_err());

        // Reject stripping credential from hop-1 while hop-2 references it
        let stripped_h1 = NewAsset::new("hop-1", "198.51.100.10", 22);
        assert!(catalog.update_asset(&h1.id, stripped_h1).await.is_err());

        // Reject deleting hop-2 while lan-postgres references it
        assert!(catalog.delete_asset(&h2.id).await.is_err());

        // Exceeding MAX_ASSET_CHAIN_HOPS (4) fails on the 5th hop
        let mut prev_id = h2.id.clone();
        for idx in 3..=4 {
            let mut next = NewAsset::new(format!("hop-{idx}"), format!("10.10.0.{idx}"), 22);
            next.credential_id = Some(cred.id.clone());
            next.via_asset_id = Some(prev_id);
            prev_id = catalog.add_asset(next).await.unwrap().id;
        }
        db_leaf_new.name = "too-deep".to_string();
        db_leaf_new.via_asset_id = Some(prev_id.clone());
        // 4 hops (hop-1 -> hop-2 -> hop-3 -> hop-4 -> too-deep) is allowed:
        let fourth = catalog.add_asset(db_leaf_new).await.unwrap();
        assert_eq!(
            catalog.resolve_asset_chain(&fourth.id).await.unwrap().len(),
            5
        );

        let mut hop5 = NewAsset::new("hop-5", "10.10.0.5", 22);
        hop5.credential_id = Some(cred.id.clone());
        hop5.via_asset_id = Some(prev_id);
        let h5 = catalog.add_asset(hop5).await.unwrap();
        let mut fifth_leaf = NewAsset::new("exceeds-depth", "10.10.0.99", 22);
        fifth_leaf.via_asset_id = Some(h5.id);
        assert!(catalog.add_asset(fifth_leaf).await.is_err());
    }

    #[tokio::test]
    async fn bootstrap_rebinds_asset_and_prunes_exclusive_local_password_credential() {
        let catalog = Catalog::in_memory().await.unwrap();
        let old_cred = catalog
            .add_credential(NewCredential {
                id: None,
                name: "new-vps-init-pass".to_string(),
                username: "root".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("enc-pass".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset_req = NewAsset::new("new-vps", "203.0.113.88", 22);
        asset_req.credential_id = Some(old_cred.id.clone());
        let asset = catalog.add_asset(asset_req).await.unwrap();

        let (updated_asset, new_cred, pruned) = catalog
            .bootstrap_bind_asset_credential(
                &asset.id,
                NewCredential {
                    id: None,
                    name: "new-vps-ed25519".to_string(),
                    username: "root".to_string(),
                    auth_type: AuthType::Key,
                    password_enc: None,
                    private_key_enc: Some("enc-key".to_string()),
                    passphrase_enc: None,
                },
                true,
            )
            .await
            .unwrap();

        assert!(pruned);
        assert_eq!(
            updated_asset.credential_id.as_deref(),
            Some(new_cred.id.as_str())
        );
        assert!(catalog
            .get_credential(&old_cred.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn probe_samples_cap_at_ring_buffer_limit() {
        let catalog = Catalog::in_memory().await.unwrap();
        let asset = catalog
            .add_asset(NewAsset::new("hk-probe", "198.51.100.25", 22))
            .await
            .unwrap();
        for idx in 0..(MAX_PROBE_SAMPLES_PER_ASSET + 10) {
            catalog
                .record_probe_sample(&asset.id, idx % 2 == 0, Some(20 + idx), None)
                .await
                .unwrap();
        }
        let samples = catalog
            .list_asset_probe_samples(&asset.id, MAX_PROBE_SAMPLES_PER_ASSET)
            .await
            .unwrap();
        assert_eq!(samples.len() as i64, MAX_PROBE_SAMPLES_PER_ASSET);
        assert_eq!(samples.first().unwrap().latency_ms, Some(30));
    }
}
